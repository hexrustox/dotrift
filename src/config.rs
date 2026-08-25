use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsString,
    fs,
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
};

use glob::{MatchOptions, Pattern};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use miette::{Result, WrapErr, miette};
use serde::Deserialize;
use templater::value::Value;

use crate::data::DataFile;

const GLOB_MATCH_OPTIONS: MatchOptions = MatchOptions {
    case_sensitive: true,
    require_literal_separator: true,
    require_literal_leading_dot: false,
};

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
    Uint(u32),
}

impl TryFrom<DeployModeRepr> for DeployMode {
    type Error = miette::Report;

    fn try_from(value: DeployModeRepr) -> Result<Self, Self::Error> {
        match value {
            DeployModeRepr::Str(value) => Self::try_from(value),
            DeployModeRepr::Uint(value) => Self::try_from(value),
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

#[derive(Debug, Clone, Copy)]
struct RuleConfig {
    deploy_type: Option<DeployType>,
    // TODO allow unset
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedPortal {
    source: PathBuf,
    target: PathBuf,
}

pub fn read(source: &Path, target_override: Option<PathBuf>) -> Result<DesiredDeployment> {
    crate::ensure_source_dir(source)?;
    let data = DataFile::read(source)?;
    let active = crate::state::StateDatabase::open_read_only()?
        .map_or_else(|| Ok(Vec::new()), |db| db.active_profiles())?;
    let context = data.context(&active).into_iter().collect::<HashMap<_, _>>();
    let config_path = source.join("dotrift.toml");
    let rendered = render_config(&config_path, &context)?;
    let config = toml::from_str::<FileConfig>(&rendered)
        .map_err(|error| miette!(error))
        .wrap_err_with(|| format!("cannot parse `{}`", config_path.display()))?;
    let target = target_override
        .or_else(|| config.target_directory.map(PathBuf::from))
        .or_else(dirs::home_dir)
        .ok_or_else(|| miette!("`HOME` is unset or empty"))?;
    if !target.is_absolute() {
        return Err(miette!("target directory must be an absolute path"));
    }
    validate_overlap(source, &target)?;
    let portals = resolve_portals(source, &config.portal)?;
    let ignore = read_ignore(source)?;
    let portals = portals
        .into_iter()
        .filter(|entry| !ignore.matched(&entry.target, false).is_ignore())
        .collect::<Vec<_>>();
    validate_targets(&portals)?;
    let rules = compile_rules(&config.rule)?;
    let entries = portals
        .into_iter()
        .map(|entry| apply_rules(entry, &rules, &target))
        .collect::<Result<Vec<_>>>()?;
    Ok(DesiredDeployment {
        target_directory: target,
        entries,
        variable_context: context,
    })
}

fn render_config(path: &Path, context: &HashMap<String, Value>) -> Result<String> {
    String::from_utf8(crate::template::render_template(path, context)?)
        .map_err(|error| miette!(error))
        .wrap_err("rendered configuration is not `UTF-8`")
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
    if path.exists() {
        match builder.add(path) {
            None => {}
            Some(error) => return Err(miette!(error)),
        }
    }
    builder.build().map_err(|error| miette!(error))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolvedKind {
    Directory,
    File,
    Dangling,
    Special,
}

type DirIdentity = (u64, u64);

fn resolve_kind(path: &Path) -> Result<ResolvedKind> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(ResolvedKind::Directory),
        Ok(metadata) if metadata.is_file() => Ok(ResolvedKind::File),
        Ok(_) => Ok(ResolvedKind::Special),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ResolvedKind::Dangling),
        Err(error) => Err(miette!(error))
            .wrap_err_with(|| format!("cannot inspect source path `{}`", path.display())),
    }
}

fn dir_identity(path: &Path) -> Result<DirIdentity> {
    let metadata = fs::metadata(path).map_err(|error| miette!(error))?;
    Ok((metadata.dev(), metadata.ino()))
}

fn walk_following_links(
    root: &Path,
    stack: &mut Vec<DirIdentity>,
    visit: &mut dyn FnMut(&Path, ResolvedKind) -> Result<()>,
) -> Result<()> {
    let identity = dir_identity(root)?;
    if stack.contains(&identity) {
        return Err(miette!(
            "symlink cycle detected while traversing `{}`",
            root.display()
        ));
    }
    stack.push(identity);
    let read_dir = fs::read_dir(root)
        .map_err(|error| miette!(error))
        .wrap_err_with(|| format!("cannot read source directory `{}`", root.display()))?;
    for entry in read_dir {
        let entry = entry.map_err(|error| miette!(error))?;
        let path = entry.path();
        match resolve_kind(&path)? {
            ResolvedKind::Directory => walk_following_links(&path, stack, visit)?,
            kind => visit(&path, kind)?,
        }
    }
    stack.pop();
    Ok(())
}

fn resolve_portals(
    source: &Path,
    portals: &BTreeMap<String, String>,
) -> Result<Vec<ResolvedPortal>> {
    let mut result = Vec::new();
    for (key, value) in portals {
        validate_relative(key, "portal source")?;
        validate_relative(value, "portal target")?;
        reject_brace_expansion(key, "portal source")?;
        reject_brace_expansion(value, "portal target")?;
        if contains_wildcard(value) {
            return Err(miette!(
                "portal target `{value}` cannot contain glob syntax"
            ));
        }
        let key = key.strip_prefix("./").unwrap_or(key);
        let value = value.strip_prefix("./").unwrap_or(value);
        if !contains_wildcard(key) {
            let path = source.join(key);
            if !path.exists() && fs::symlink_metadata(&path).is_err() {
                return Err(miette!("literal portal source `{key}` does not exist"));
            }
            match resolve_kind(&path)? {
                ResolvedKind::File => push_deployable(&mut result, &path, Path::new(value))?,
                ResolvedKind::Directory => {
                    let mut stack = Vec::new();
                    walk_following_links(&path, &mut stack, &mut |child, kind| match kind {
                        ResolvedKind::File => {
                            let relative = child.strip_prefix(&path).map_err(|_| {
                                miette!(
                                    "entry `{}` is outside the portal source `{}`",
                                    child.display(),
                                    path.display()
                                )
                            })?;
                            push_deployable(&mut result, child, &Path::new(value).join(relative))
                        }
                        ResolvedKind::Dangling => {
                            Err(miette!("dangling symlink `{}`", child.display()))
                        }
                        ResolvedKind::Special => Err(miette!(
                            "source path `{}` is not a regular file, symlink to a regular file, or directory",
                            child.display()
                        )),
                        ResolvedKind::Directory => {
                            unreachable!("directories are descended, never visited")
                        }
                    })?;
                }
                ResolvedKind::Dangling => {
                    return Err(miette!("dangling symlink `{}`", path.display()));
                }
                ResolvedKind::Special => {
                    return Err(miette!(
                        "source path `{}` is not a regular file, symlink to a regular file, or directory",
                        path.display()
                    ));
                }
            }
            continue;
        }

        let strip = wildcard_prefix(key);
        let pattern = Pattern::new(key)
            .map_err(|error| miette!("invalid portal pattern `{key}` because {error}"))?;
        let mut stack = Vec::new();
        walk_following_links(source, &mut stack, &mut |path, kind| {
            let relative = path.strip_prefix(source).map_err(|_| {
                miette!(
                    "entry `{}` is outside the source directory `{}`",
                    path.display(),
                    source.display()
                )
            })?;
            if !pattern.matches_path_with(relative, GLOB_MATCH_OPTIONS) {
                return Ok(());
            }
            match kind {
                ResolvedKind::File => {
                    let remainder = relative.strip_prefix(&strip).unwrap_or(relative);
                    push_deployable(&mut result, path, &PathBuf::from(value).join(remainder))
                }
                ResolvedKind::Dangling => Err(miette!("dangling symlink `{}`", path.display())),
                ResolvedKind::Special => Err(miette!(
                    "source path `{}` is not a regular file, symlink to a regular file, or directory",
                    path.display()
                )),
                ResolvedKind::Directory => unreachable!("directories are descended, never visited"),
            }
        })?;
    }
    Ok(result)
}

fn push_deployable(result: &mut Vec<ResolvedPortal>, source: &Path, target: &Path) -> Result<()> {
    if !fs::metadata(source).is_ok_and(|meta| meta.is_file()) {
        return Err(miette!(
            "source path `{}` is not a regular file or symlink to a regular file",
            source.display()
        ));
    }
    result.push(ResolvedPortal {
        source: source.to_path_buf(),
        target: target.to_path_buf(),
    });
    Ok(())
}

fn contains_wildcard(value: &str) -> bool {
    value
        .bytes()
        .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b']'))
}

fn reject_brace_expansion(value: &str, what: &str) -> Result<()> {
    if value.bytes().any(|byte| byte == b'{' || byte == b'}') {
        return Err(miette!(
            "unsupported pattern syntax in {what} `{value}`: brace expansion is not supported"
        ));
    }
    Ok(())
}

fn wildcard_prefix(pattern: &str) -> PathBuf {
    let mut prefix = PathBuf::new();
    for component in Path::new(pattern).components() {
        let value = component.as_os_str().to_string_lossy();
        if contains_wildcard(&value) {
            break;
        }
        prefix.push(value.as_ref());
    }
    prefix
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
    if mode.is_some() && deploy_type == DeployType::Symlink {
        return Err(miette!(
            "conflicting rules for `{}`: `mode` is set but the effective `type` is `symlink`",
            entry.target.display()
        ));
    }
    Ok(DeploymentEntry {
        source_path: entry.source,
        target_path: target_root.join(entry.target),
        deploy_type,
        mode,
    })
}

// TODO normalize
fn validate_relative(value: &str, what: &str) -> Result<()> {
    if value.is_empty() || Path::new(value).is_absolute() {
        return Err(miette!("invalid {what} path `{value}`"));
    }
    for (index, component) in Path::new(value).components().enumerate() {
        if matches!(component, Component::CurDir | Component::ParentDir)
            && !(index == 0 && component == Component::CurDir)
        {
            return Err(miette!("invalid {what} path `{value}`"));
        }
    }
    Ok(())
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

    #[test_case("" => matches Ok(RuleConfig { deploy_type: None, mode: None }) ; "empty_table_means_no_properties")]
    #[test_case("type = \"symlink\"" => matches Ok(RuleConfig { deploy_type: Some(DeployType::Symlink), mode: None }) ; "symlink_type_parsed")]
    #[test_case("type = \"copy\"" => matches Ok(RuleConfig { deploy_type: Some(DeployType::Copy), mode: None }) ; "copy_type_parsed")]
    #[test_case("type = \"template\"" => matches Ok(RuleConfig { deploy_type: Some(DeployType::Template), mode: None }) ; "template_type_parsed")]
    #[test_case("mode = \"644\"" => matches Ok(RuleConfig { deploy_type: None, mode: Some(DeployMode(0o644)) }) ; "mode_parsed_without_type")]
    #[test_case("type = \"copy\"\nmode = \"644\"" => matches Ok(RuleConfig { deploy_type: Some(DeployType::Copy), mode: Some(DeployMode(0o644)) }) ; "mode_combines_with_copy")]
    fn deserializes_valid_rule_config(toml: &str) -> Result<RuleConfig, toml::de::Error> {
        toml::from_str(toml)
    }

    #[test_case("type = \"symlink\"\nmode = \"644\"" => matches Err(e) if e.to_string().contains("cannot be used") ; "mode_rejected_with_symlink")]
    #[test_case("bogus = 1" => matches Err(_) ; "unknown_property_rejected")]
    #[test_case("type = \"hardlink\"" => matches Err(_) ; "invalid_type_rejected")]
    #[test_case("mode = \"888\"" => matches Err(_) ; "invalid_mode_rejected")]
    fn rejects_invalid_rule_config(toml: &str) -> Result<RuleConfig, toml::de::Error> {
        toml::from_str(toml)
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
            fs::create_dir(t.join("a")).expect("cannot create temp dir");
            fs::create_dir(t.join("b")).expect("cannot create temp dir");
            (t.join("a"), t.join("b"))
        } => true;
        "disjoint_sibling_roots_do_not_overlap"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("target/source")).expect("cannot create temp dirs");
            (t.join("target/source"), t.join("target"))
        } => true;
        "source_nested_inside_target_does_not_overlap"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("root")).expect("cannot create temp dir");
            (t.join("root"), t.join("root"))
        } => false;
        "equal_roots_overlap"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("root/target")).expect("cannot create temp dirs");
            (t.join("root"), t.join("root/target"))
        } => false;
        "target_inside_source_overlaps"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("root/x/y/z")).expect("cannot create temp dirs");
            (t.join("root"), t.join("root/x/y/z"))
        } => false;
        "deeply_nested_target_inside_source_overlaps"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("real")).expect("cannot create temp dir");
            std::os::unix::fs::symlink(t.join("real"), t.join("link")).expect("cannot create symlink");
            (t.join("real"), t.join("link"))
        } => false;
        "target_symlinked_onto_source_overlaps"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("real/nested")).expect("cannot create temp dirs");
            std::os::unix::fs::symlink(t.join("real/nested"), t.join("link")).expect("cannot create symlink");
            (t.join("real"), t.join("link"))
        } => false;
        "target_symlink_pointing_into_source_overlaps"
    )]
    fn roots_satisfy_overlap_rule(setup: impl Fn(&Path) -> (PathBuf, PathBuf)) -> bool {
        let dir = tempdir().expect("cannot create temp dir");
        let (source, target) = setup(dir.path());
        validate_overlap(&source, &target).is_ok()
    }

    macro_rules! portal_map {
        ($($source:literal => $target:literal),* $(,)?) => {
            BTreeMap::from([
                $(($source.to_string(), $target.to_string())),*
            ])
        };
    }

    macro_rules! resolved_list {
        ($($source:literal => $target:literal),* $(,)?) => {
            {
                let vec: Vec<ResolvedPortal> = vec![
                    $(ResolvedPortal {
                        source: PathBuf::from($source),
                        target: PathBuf::from($target),
                    }),*
                ];
                vec
            }
        };
    }

    #[test_case(|_t| portal_map!() => resolved_list!(); "empty_portals_produce_no_entries")]
    #[test_case(
        |t| {
            fs::write(t.join("vimrc"), b"set").unwrap();
            portal_map!("vimrc" => ".vimrc")
        } => resolved_list!("vimrc" => ".vimrc");
        "literal_file_maps_to_exact_target"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a")).unwrap();
            fs::create_dir_all(t.join("sub/a")).unwrap();
            fs::write(t.join("a/b"), b"a").unwrap();
            fs::write(t.join("sub/a/b"), b"b").unwrap();
            portal_map!("a/b" => "dir")
        } => resolved_list!(
            "a/b" => "dir",
        );
        "literal_portal_is_anchored_does_not_match_nested_path"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("vimrc"), b"set").unwrap();
            portal_map!("./vimrc" => "./.vimrc")
        } => resolved_list!("vimrc" => ".vimrc");
        "dot_slash_prefix_is_stripped"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("nvim/lua")).unwrap();
            fs::write(t.join("nvim/init.lua"), b"-- lua").unwrap();
            fs::write(t.join("nvim/lua/mappings.lua"), b"-- mappings").unwrap();
            portal_map!("nvim" => ".config/nvim")
        } => resolved_list!(
            "nvim/init.lua" => ".config/nvim/init.lua",
            "nvim/lua/mappings.lua" => ".config/nvim/lua/mappings.lua"
        );
        "directory_portal_appends_relative_suffix"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("config/sub")).unwrap();
            fs::write(t.join("config/one.toml"), b"a").unwrap();
            fs::write(t.join("config/sub/two.toml"), b"b").unwrap();
            portal_map!("config/*.toml" => ".config")
        } => resolved_list!(
            "config/one.toml" => ".config/one.toml"
        );
        "wildcard_pattern_does_not_cross_directory"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("config/sub")).unwrap();
            fs::write(t.join("config/one.toml"), b"a").unwrap();
            fs::write(t.join("config/sub/two.toml"), b"b").unwrap();
            portal_map!("config/**/*.toml" => ".config")
        } => resolved_list!(
            "config/one.toml" => ".config/one.toml",
            "config/sub/two.toml" => ".config/sub/two.toml"
        );
        "recursive_wildcard_pattern_appends_remainder"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("empty")).unwrap();
            portal_map!("empty" => ".config/empty")
        } => resolved_list!();
        "empty_directory_expands_to_nothing"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("real"), b"content").unwrap();
            std::os::unix::fs::symlink(t.join("real"), t.join("link")).unwrap();
            portal_map!("link" => ".link")
        } => resolved_list!("link" => ".link");
        "symlink_to_file_maps_to_exact_target"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("real")).unwrap();
            fs::write(t.join("real/a"), b"a").unwrap();
            fs::write(t.join("real/b"), b"b").unwrap();
            std::os::unix::fs::symlink(t.join("real"), t.join("dirlink")).unwrap();
            portal_map!("dirlink" => ".config")
        } => resolved_list!(
            "dirlink/a" => ".config/a",
            "dirlink/b" => ".config/b"
        );
        "symlink_to_directory_maps_contents"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("data"), b"content").unwrap();
            std::os::unix::fs::symlink(t.join("data"), t.join("link.lnk")).unwrap();
            portal_map!("*.lnk" => ".dots")
        } => resolved_list!("link.lnk" => ".dots/link.lnk");
        "symlink_file_matched_by_wildcard"
    )]
    #[test_case(|_t| portal_map!("" => ".vimrc") => panics "invalid portal source path"; "empty_source_is_rejected")]
    #[test_case(|_t| portal_map!("vimrc" => "/home/.vimrc") => panics "invalid portal target path"; "absolute_target_is_rejected")]
    #[test_case(|_t| portal_map!("a/../vimrc" => ".vimrc") => panics "a/../vimrc"; "parent_component_in_source_is_rejected")]
    #[test_case(|_t| portal_map!("{a,b}" => ".a") => panics "in portal source"; "brace_expansion_in_source_is_rejected")]
    #[test_case(|_t| portal_map!("a" => ".{a,b}") => panics "in portal target"; "brace_expansion_in_target_is_rejected")]
    #[test_case(|_t| portal_map!("a" => "*.conf") => panics "cannot contain glob syntax"; "wildcard_target_is_rejected")]
    #[test_case(|_t| portal_map!("missing" => ".missing") => panics "does not exist"; "missing_literal_source_is_rejected")]
    #[test_case(
        |t| {
            std::os::unix::fs::symlink(t.join("nowhere"), t.join("link")).unwrap();
            portal_map!("link" => "link")
        } => panics "dangling symlink";
        "dangling_literal_source_is_rejected"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("dir")).unwrap();
            fs::write(t.join("dir/ok"), b"ok").unwrap();
            std::os::unix::fs::symlink(t.join("nowhere"), t.join("dir/broken")).unwrap();
            portal_map!("dir" => ".config")
        } => panics "dangling symlink";
        "dangling_symlink_within_directory_is_rejected"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("ok"), b"ok").unwrap();
            std::os::unix::fs::symlink(t.join("nowhere"), t.join("broken")).unwrap();
            portal_map!("*" => ".dots")
        } => panics "dangling symlink";
        "dangling_symlink_matched_by_wildcard_is_rejected"
    )]
    #[test_case(
        |t| {
            std::os::unix::fs::symlink(t.join("a"), t.join("a")).unwrap();
            portal_map!("a" => ".a")
        } => panics "cannot inspect source path";
        "self_referential_symlink_is_rejected"
    )]
    #[test_case(
        |t| {
            std::os::unix::fs::symlink(t.join("b"), t.join("a")).unwrap();
            std::os::unix::fs::symlink(t.join("a"), t.join("b")).unwrap();
            portal_map!("a" => ".a")
        } => panics "cannot inspect source path";
        "mutual_symlink_cycle_is_rejected"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("dir")).unwrap();
            fs::write(t.join("dir/real"), b"x").unwrap();
            std::os::unix::fs::symlink(t.join("dir"), t.join("dir/loop")).unwrap();
            portal_map!("dir" => ".config")
        } => panics "symlink cycle detected";
        "symlink_cycle_within_directory_is_rejected"
    )]
    fn expands_portals_to_resolved_entries(
        setup: impl Fn(&Path) -> BTreeMap<String, String>,
    ) -> Vec<ResolvedPortal> {
        let dir = tempdir().expect("cannot create temp dir");
        let mut entries = resolve_portals(dir.path(), &setup(dir.path()))
            .unwrap()
            .into_iter()
            .map(|entry| ResolvedPortal {
                source: entry.source.strip_prefix(dir.path()).unwrap().to_path_buf(),
                target: entry.target,
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.source.clone());
        entries
    }

    #[test_case(resolved_list!(); "empty_target_set_is_valid")]
    #[test_case(resolved_list!("a" => ".config/a") ; "single_distinct_target_is_valid")]
    #[test_case(resolved_list!("a" => ".config/a", "b" => ".config/b") ; "distinct_targets_under_shared_directory")]
    #[test_case(resolved_list!("a" => "x/y", "b" => "x/z") ; "sibling_targets_under_nested_directory")]
    #[test_case(resolved_list!("a" => "deep/a/b/c", "b" => "deep/d/e") ; "deeply_nested_distinct_targets")]
    #[test_case(resolved_list!("a" => ".config", "b" => ".config") => panics "collision at"; "identical_targets_collide")]
    #[test_case(resolved_list!("a" => "x", "b" => "x/y") => panics "structural conflict"; "file_target_blocked_by_nested_target")]
    #[test_case(resolved_list!("a" => "x/y", "b" => "x") => panics "structural conflict"; "nested_target_blocked_by_file_target")]
    #[test_case(resolved_list!("a" => "x/y/z", "b" => "x/y") => panics "structural conflict"; "deeply_nested_target_blocked_by_file_target")]
    fn validates_targets_are_structurally_disjoint(entries: Vec<ResolvedPortal>) {
        validate_targets(&entries).unwrap()
    }

    macro_rules! resolved {
        ($source:literal, $target:literal) => {
            ResolvedPortal {
                source: PathBuf::from($source),
                target: PathBuf::from($target),
            }
        };
    }

    macro_rules! rule_map {
        ($($source:literal => $config:expr),* $(,)?) => {
            indexmap::IndexMap::from_iter([
                $((Pattern::new($source).unwrap(), $config)),*
            ])
        };
    }

    macro_rules! rule_config {
        ($deploy:ident) => {
            RuleConfig {
                deploy_type: Some(DeployType::$deploy),
                mode: None,
            }
        };
        ($deploy:ident, $mode:literal) => {
            RuleConfig {
                deploy_type: Some(DeployType::$deploy),
                mode: Some(DeployMode($mode)),
            }
        };
    }

    #[test_case(
        resolved!("vimrc", ".vimrc"), rule_map!()
        => deploy_entry!("vimrc", ".vimrc", Symlink);
        "no_rules_defaults_to_symlink_without_mode"
    )]
    #[test_case(
        resolved!("vimrc", ".vimrc"), rule_map!("*" => rule_config!(Copy))
        => deploy_entry!("vimrc", ".vimrc", Copy);
        "copy_rule_overrides_default_symlink"
    )]
    #[test_case(
        resolved!("vimrc", ".vimrc"), rule_map!("*" => rule_config!(Template))
        => deploy_entry!("vimrc", ".vimrc", Template);
        "template_rule_overrides_default_symlink"
    )]
    #[test_case(
        resolved!("vimrc", ".vimrc"), rule_map!("*" => rule_config!(Copy, 0o644))
        => deploy_entry!("vimrc", ".vimrc", Copy, 0o644);
        "mode_is_applied_with_copy"
    )]
    #[test_case(
        resolved!("vimrc", ".vimrc"), rule_map!("*.conf" => rule_config!(Copy, 0o600))
        => deploy_entry!("vimrc", ".vimrc", Symlink);
        "unmatched_rule_is_ignored"
    )]
    #[test_case(
        resolved!("vimrc", ".vimrc"), rule_map!("*" => rule_config!(Template), "*.vimrc" => rule_config!(Copy))
        => deploy_entry!("vimrc", ".vimrc", Copy);
        "last_matching_rule_wins_type"
    )]
    #[test_case(
        resolved!("vimrc", ".vimrc"), rule_map!("*" => rule_config!(Copy, 0o600), "*.vimrc" => rule_config!(Template))
        => deploy_entry!("vimrc", ".vimrc", Template, 0o600);
        "mode_survives_later_type_only_rule"
    )]
    #[test_case(
        resolved!("deep", "nested/x"), rule_map!("nested/*" => rule_config!(Template))
        => deploy_entry!("deep", "nested/x", Template);
        "pattern_applies_to_nested_target"
    )]
    #[test_case(
        resolved!("vimrc", ".vimrc"), rule_map!("*" => rule_config!(Copy, 0o644), "*.vimrc" => rule_config!(Symlink))
        => panics "conflicting rules";
        "mode_with_effective_symlink_is_rejected"
    )]
    #[test_case(
        resolved!("src", "a/b"), rule_map!("a/b" => rule_config!(Copy))
        => deploy_entry!("src", "a/b", Copy);
        "literal_rule_matches_exact_target"
    )]
    #[test_case(
        resolved!("src", "sub/a/b"), rule_map!("a/b" => rule_config!(Copy))
        => deploy_entry!("src", "sub/a/b", Symlink);
        "literal_rule_does_not_match_nested_prefix"
    )]
    #[test_case(
        resolved!("src", "a/b/c"), rule_map!("a/b" => rule_config!(Copy))
        => deploy_entry!("src", "a/b/c", Symlink);
        "literal_rule_does_not_match_child"
    )]
    #[test_case(
        resolved!("src", "a/b"), rule_map!("a/*" => rule_config!(Copy))
        => deploy_entry!("src", "a/b", Copy);
        "wildcard_single_component_matches_direct_child"
    )]
    #[test_case(
        resolved!("src", "a/b/c"), rule_map!("a/*" => rule_config!(Copy))
        => deploy_entry!("src", "a/b/c", Symlink);
        "wildcard_single_component_does_not_cross_separator"
    )]
    #[test_case(
        resolved!("src", "a/b"), rule_map!("*" => rule_config!(Copy))
        => deploy_entry!("src", "a/b", Symlink);
        "star_does_not_match_nested_target"
    )]
    #[test_case(
        resolved!("src", "a/b/c"), rule_map!("a/**" => rule_config!(Copy))
        => deploy_entry!("src", "a/b/c", Copy);
        "recursive_wildcard_matches_deep_target"
    )]
    #[test_case(
        resolved!("src", "a/b"), rule_map!("a/**" => rule_config!(Copy))
        => deploy_entry!("src", "a/b", Copy);
        "recursive_wildcard_matches_direct_child"
    )]
    fn applies_matching_rules(
        entry: ResolvedPortal,
        rules: indexmap::IndexMap<Pattern, RuleConfig>,
    ) -> DeploymentEntry {
        let dir = tempdir().expect("cannot create temp dir");
        let actual = match apply_rules(entry, &rules, dir.path()) {
            Ok(actual) => actual,
            Err(error) => panic!("apply_rules failed: {error}"),
        };
        let mut actual = actual;
        actual.target_path = actual
            .target_path
            .strip_prefix(dir.path())
            .expect("target path is under the temp dir")
            .to_path_buf();
        actual
    }
}
