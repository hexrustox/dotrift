use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use glob::{MatchOptions, Pattern};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use miette::{Result, WrapErr, miette};
use serde::Deserialize;
use templater::value::Value;

use crate::{
    platform::{Environment, ensure_source_dir},
    report::Reporter,
};

pub(crate) mod data;
pub mod global;
mod portals;

pub(crate) use data::DataFile;
pub use global::{DiffCommand, GlobalConfig, PagerCommand};

const GLOB_MATCH_OPTIONS: MatchOptions = MatchOptions {
    case_sensitive: true,
    require_literal_separator: true,
    require_literal_leading_dot: false,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedPortal {
    source: PathBuf,
    target: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeployType {
    Symlink,
    Copy,
    Template,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "DeployModeRepr")]
pub struct DeployMode(pub u32);

impl From<DeployMode> for u32 {
    fn from(value: DeployMode) -> Self {
        value.0
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DeployModeRepr {
    Str(String),
    Int(i64),
}

impl TryFrom<DeployModeRepr> for DeployMode {
    type Error = miette::Report;

    fn try_from(value: DeployModeRepr) -> Result<Self, Self::Error> {
        match value {
            DeployModeRepr::Str(value) => Self::try_from(value),
            DeployModeRepr::Int(value) => Self::try_from(value),
        }
    }
}

impl TryFrom<String> for DeployMode {
    type Error = miette::Report;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() != 3 || !value.bytes().all(|byte| (b'0'..=b'7').contains(&byte)) {
            return Err(miette!("invalid mode `{value}`"));
        }
        let mode = value
            .bytes()
            .fold(0u32, |mode, byte| (mode << 3) | u32::from(byte - b'0'));
        Ok(Self(mode))
    }
}

impl TryFrom<i64> for DeployMode {
    type Error = miette::Report;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if !matches!(value, 0..=0o777) {
            return Err(miette!(
                "invalid mode `{}`",
                if value < 0 {
                    value.to_string()
                } else {
                    format!("{value:o}")
                }
            ));
        }
        Ok(Self(value as u32))
    }
}

impl TryFrom<u32> for DeployMode {
    type Error = miette::Report;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        if value > 0o777 {
            return Err(miette!("invalid mode `{value:o}`"));
        }
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentEntry {
    pub source_path: PathBuf,
    pub target_path: PathBuf,
    pub deploy_type: DeployType,
    pub mode: Option<DeployMode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredDeployment {
    pub target_directory: PathBuf,
    pub entries: Vec<DeploymentEntry>,
    pub variable_context: HashMap<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    #[serde(rename = "target-directory")]
    target_directory: Option<String>,
    #[serde(default)]
    portal: BTreeMap<String, String>,
    #[serde(default)]
    rule: indexmap::IndexMap<String, RuleConfig>,
}

pub fn read(
    source: &Path,
    target_override: Option<PathBuf>,
    env: &Environment,
    color: bool,
) -> Result<DesiredDeployment> {
    ensure_source_dir(source)?;
    let data = DataFile::read(source)?;
    // The variable context is resolved once per run (`spec/CONTEXT.md`); a
    // missing state database contributes no active profiles (`spec/core.md §
    // State database`).
    let active = crate::state::load_active_profiles(env)?;
    let context = data.context(&active).into_iter().collect::<HashMap<_, _>>();
    let config_path = source.join("dotrift.toml");
    let rendered = render_config(&config_path, &context)?;
    let config = toml::from_str::<FileConfig>(&rendered)
        .map_err(|error| miette!(error))
        .wrap_err_with(|| format!("cannot parse `{}`", config_path.display()))?;
    let target = match target_override.or_else(|| config.target_directory.map(PathBuf::from)) {
        Some(target) => target,
        None => env.default_target_dir()?,
    };
    if !target.is_absolute() {
        return Err(miette!("target directory must be an absolute path"));
    }
    validate_overlap(source, &target)?;
    // The Desired deployment chain owns one invariant: Ignore patterns and
    // Rules match target paths only (`spec/CONTEXT.md` § Ignore pattern,
    // § Rule; ADR-0002). Portal resolution produces target paths, filtering
    // removes ignored targets, validation rejects Collisions and Structural
    // conflicts among the survivors, and Rules decorate them. The order is
    // enforced here by construction: an ignored entry never reaches
    // validation, so it never causes a Collision.
    let portals = portals::resolve_portals(source, &config.portal)?;
    let ignore = read_ignore(source)?;
    let portals = portals
        .into_iter()
        .filter(|entry| !ignore.matched(&entry.target, false).is_ignore())
        .collect::<Vec<_>>();
    validate_targets(&portals)?;
    let rules = compile_rules(&config.rule)?;
    let entries = portals
        .into_iter()
        .map(|entry| apply_rules(entry, &rules, &target, color))
        .collect::<Result<Vec<_>>>()?;
    Ok(DesiredDeployment {
        target_directory: target,
        entries,
        variable_context: context,
    })
}

fn render_config(path: &Path, context: &HashMap<String, Value>) -> Result<String> {
    String::from_utf8(crate::render::render_template(path, context)?)
        .map_err(|error| miette!(error))
        .wrap_err("rendered configuration is not `UTF-8`")
}

fn validate_overlap(source: &Path, target: &Path) -> Result<()> {
    let source = resolve_for_comparison(source)?;
    let target = resolve_for_comparison(target)?;
    if source == target || target.starts_with(&source) {
        return Err(miette!("source and target directories overlap"));
    }
    Ok(())
}

fn resolve_for_comparison(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return fs::canonicalize(path).map_err(|error| miette!(error));
    }
    let mut missing = Vec::new();
    let mut existing = path.to_path_buf();
    while !existing.exists() {
        missing.push(
            existing
                .file_name()
                .ok_or_else(|| miette!("cannot resolve path `{}`", path.display()))?
                .to_owned(),
        );
        existing.pop();
    }
    let mut resolved = fs::canonicalize(existing).map_err(|error| miette!(error))?;
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

fn validate_relative(value: &str, what: &str) -> Result<()> {
    if value.is_empty() || Path::new(value).is_absolute() {
        return Err(miette!("invalid {what} path `{value}`"));
    }
    for (index, component) in value.split('/').enumerate() {
        let valid = match component {
            "" | ".." => false,
            "." => index == 0,
            _ => true,
        };
        if !valid {
            return Err(miette!("invalid {what} path `{value}`"));
        }
    }
    Ok(())
}

fn reject_brace_expansion(value: &str, what: &str) -> Result<()> {
    if value.bytes().any(|byte| byte == b'{' || byte == b'}') {
        return Err(miette!(
            "unsupported pattern syntax in {what} `{value}`: brace expansion is not supported"
        ));
    }
    Ok(())
}

fn read_ignore(source: &Path) -> Result<Gitignore> {
    let mut builder = GitignoreBuilder::new(source);
    builder
        .add_line(None, "/dotrift.toml")
        .map_err(|error| miette!(error))?;
    builder
        .add_line(None, "/dotrift_data.toml")
        .map_err(|error| miette!(error))?;
    builder
        .add_line(None, "/.dotriftignore")
        .map_err(|error| miette!(error))?;
    let path = source.join(".dotriftignore");
    match fs::File::open(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Missing, or a dangling symlink: "no ignore patterns".
        }
        Err(error) => {
            return Err(miette!(error))
                .wrap_err_with(|| format!("cannot read `{}`", path.display()));
        }
        Ok(_) => match builder.add(path) {
            None => {}
            Some(error) => return Err(miette!(error)),
        },
    }
    builder.build().map_err(|error| miette!(error))
}

fn validate_targets(entries: &[ResolvedPortal]) -> Result<()> {
    let mut tree = Node::Dir(BTreeMap::new());
    for entry in entries {
        tree.insert(&entry.target, &entry.source)?;
    }
    Ok(())
}

enum Node {
    Dir(BTreeMap<OsString, Node>),
    File(PathBuf),
}

impl Default for Node {
    fn default() -> Self {
        Self::Dir(BTreeMap::new())
    }
}

impl Node {
    fn insert(&mut self, target: &Path, source: &Path) -> Result<()> {
        let mut node = self;
        let mut consumed = PathBuf::new();
        let mut components = target.components().peekable();
        while let Some(component) = components.next() {
            let name = component.as_os_str().to_owned();
            match node {
                Node::File(_) => {
                    return Err(miette!(
                        "structural conflict between `{}` and `{}`",
                        consumed.display(),
                        target.display()
                    ));
                }
                Node::Dir(children) => {
                    consumed.push(&name);
                    let child = children.entry(name).or_default();
                    if components.peek().is_none() {
                        if let Node::File(existing) = child {
                            return Err(miette!(
                                "collision at `{}` between `{}` and `{}`",
                                target.display(),
                                existing.display(),
                                source.display()
                            ));
                        }
                        if matches!(child, Node::Dir(children) if children.is_empty()) {
                            *child = Node::File(source.to_path_buf());
                            return Ok(());
                        }
                        let descendant = consumed.join(child.first_file_path());
                        return Err(miette!(
                            "structural conflict between `{}` and `{}`",
                            target.display(),
                            descendant.display()
                        ));
                    }
                    node = child;
                }
            }
        }
        unreachable!("validated targets are non-empty relative paths")
    }

    fn first_file_path(&self) -> PathBuf {
        match self {
            Node::File(_) => PathBuf::new(),
            Node::Dir(children) => {
                let (name, child) = children.iter().next().unwrap_or_else(|| {
                    unreachable!("caller guarantees the directory has children")
                });
                PathBuf::from(name).join(child.first_file_path())
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RuleConfig {
    deploy_type: Option<DeployType>,
    mode: Option<DeployMode>,
}

impl<'de> serde::Deserialize<'de> for RuleConfig {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            #[serde(rename = "type")]
            deploy_type: Option<DeployType>,
            mode: Option<DeployMode>,
        }
        let raw = Raw::deserialize(deserializer)?;
        if raw.mode.is_some() && raw.deploy_type == Some(DeployType::Symlink) {
            return Err(serde::de::Error::custom(
                "`mode` cannot be used with `type` `symlink`",
            ));
        }
        Ok(RuleConfig {
            deploy_type: raw.deploy_type,
            mode: raw.mode,
        })
    }
}

fn compile_rules(
    rules: &indexmap::IndexMap<String, RuleConfig>,
) -> Result<indexmap::IndexMap<Pattern, RuleConfig>> {
    let mut compiled = indexmap::IndexMap::with_capacity(rules.len());
    for (pattern, rule) in rules {
        validate_relative(pattern, "rule")?;
        reject_brace_expansion(pattern, "rule")?;
        let pattern = Pattern::new(pattern.strip_prefix("./").unwrap_or(pattern))
            .map_err(|error| miette!("invalid rule pattern `{pattern}` because {error}"))?;
        compiled.insert(pattern, *rule);
    }
    Ok(compiled)
}

fn apply_rules(
    entry: ResolvedPortal,
    rules: &indexmap::IndexMap<Pattern, RuleConfig>,
    target_root: &Path,
    color: bool,
) -> Result<DeploymentEntry> {
    let mut deploy_type = DeployType::Symlink;
    let mut mode = None;
    for (pattern, rule) in rules {
        if pattern.matches_path_with(&entry.target, GLOB_MATCH_OPTIONS) {
            if let Some(value) = rule.deploy_type {
                deploy_type = value;
            }
            if let Some(value) = &rule.mode {
                mode = Some(*value);
            }
        }
    }
    if deploy_type == DeployType::Symlink {
        if mode.is_some() {
            Reporter::always(color).warning(format_args!(
                "ignoring `mode` for `{}`: effective `type` is `symlink`",
                entry.target.display()
            ));
        }
        mode = None;
    }
    Ok(DeploymentEntry {
        source_path: entry.source,
        target_path: target_root.join(entry.target),
        deploy_type,
        mode,
    })
}

#[cfg(any(test, feature = "testing"))]
#[macro_export]
macro_rules! deploy_entry {
    ($source:expr, $target:expr, $deploy:ident) => {
        $crate::config::DeploymentEntry {
            source_path: std::path::PathBuf::from($source),
            target_path: std::path::PathBuf::from($target),
            deploy_type: $crate::config::DeployType::$deploy,
            mode: None,
        }
    };
    ($source:expr, $target:expr, $deploy:ident, $mode:expr) => {
        $crate::config::DeploymentEntry {
            source_path: std::path::PathBuf::from($source),
            target_path: std::path::PathBuf::from($target),
            deploy_type: $crate::config::DeployType::$deploy,
            mode: Some($crate::config::DeployMode($mode)),
        }
    };
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use tempfile::tempdir;
    use test_case::test_case;

    use super::*;

    #[test_case("000" => 0)]
    #[test_case("100" => 64)]
    #[test_case("017" => 15)]
    #[test_case("770" => 504)]
    #[test_case("777" => 511)]
    fn parses_three_octal_digits_to_their_value(s: &str) -> u32 {
        u32::from(DeployMode::try_from(s.to_string()).expect("valid octal mode string"))
    }

    #[test_case("" => matches Err(_) ; "empty_rejected")]
    #[test_case("7" => matches Err(_) ; "single_digit_rejected")]
    #[test_case("77" => matches Err(_) ; "two_digits_rejected")]
    #[test_case("7777" => matches Err(_) ; "four_digits_rejected")]
    #[test_case("a77" => matches Err(_) ; "non_octal_first_byte_rejected")]
    #[test_case("7a7" => matches Err(_) ; "non_octal_middle_byte_rejected")]
    #[test_case("77a" => matches Err(_) ; "non_octal_last_byte_rejected")]
    #[test_case("779" => matches Err(_) ; "digit_nine_rejected")]
    #[test_case("888" => matches Err(_) ; "digit_eight_rejected")]
    fn string_not_of_three_octal_digits_is_rejected(s: &str) -> Result<DeployMode, miette::Report> {
        DeployMode::try_from(s.to_string())
    }

    #[test_case(0 => 0 ; "zero")]
    #[test_case(0o100 => 64 ; "middle_value")]
    #[test_case(0o777 => 511 ; "max_allowed")]
    fn u32_in_octal_range_converts_to_its_value(value: u32) -> u32 {
        u32::from(DeployMode::try_from(value).expect("valid octal mode value"))
    }

    #[test_case(0o1000 => matches Err(_) ; "just_above_max_rejected")]
    #[test_case(u32::MAX => matches Err(_) ; "max_u32_rejected")]
    fn u32_out_of_octal_range_is_rejected(value: u32) -> Result<DeployMode, miette::Report> {
        DeployMode::try_from(value)
    }

    proptest! {
        #[test]
        fn parses_exactly_when_string_is_three_octal_digits(s in "[0-9a-zA-Z]{0,6}") {
            let is_three_octal = s.len() == 3 && s.bytes().all(|b| (b'0'..=b'7').contains(&b));
            let expected = is_three_octal.then(|| u32::from_str_radix(&s, 8).unwrap());
            let actual = DeployMode::try_from(s).ok().map(u32::from);
            prop_assert_eq!(actual, expected);
        }
    }

    #[test_case(
        |t| {
            fs::create_dir(t.join("dir1")).expect("cannot create temp dir");
            fs::create_dir(t.join("dir2")).expect("cannot create temp dir");
            (t.join("dir1"), t.join("dir2"))
        } => true;
        "disjoint_sibling_roots_do_not_overlap"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("dir1/sub1")).expect("cannot create temp dirs");
            (t.join("dir1/sub1"), t.join("dir1"))
        } => true;
        "source_nested_inside_target_does_not_overlap"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("dir1")).expect("cannot create temp dir");
            (t.join("dir1"), t.join("dir1"))
        } => false;
        "equal_roots_overlap"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("dir1/sub1")).expect("cannot create temp dirs");
            (t.join("dir1"), t.join("dir1/sub1"))
        } => false;
        "target_inside_source_overlaps"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("dir1/sub1/dir2/sub2")).expect("cannot create temp dirs");
            (t.join("dir1"), t.join("dir1/sub1/dir2/sub2"))
        } => false;
        "deeply_nested_target_inside_source_overlaps"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("dir1")).expect("cannot create temp dir");
            std::os::unix::fs::symlink(t.join("dir1"), t.join("link1")).expect("cannot create symlink");
            (t.join("dir1"), t.join("link1"))
        } => false;
        "target_symlinked_onto_source_overlaps"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("dir1/sub1")).expect("cannot create temp dirs");
            std::os::unix::fs::symlink(t.join("dir1/sub1"), t.join("link1")).expect("cannot create symlink");
            (t.join("dir1"), t.join("link1"))
        } => false;
        "target_symlink_pointing_into_source_overlaps"
    )]
    fn roots_satisfy_overlap_rule(setup: impl Fn(&Path) -> (PathBuf, PathBuf)) -> bool {
        let dir = tempdir().expect("cannot create temp dir");
        let (source, target) = setup(dir.path());
        validate_overlap(&source, &target).is_ok()
    }
}
