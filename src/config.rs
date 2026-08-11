use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsString,
    fs,
    path::{Component, Path, PathBuf},
};

use glob::Pattern;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use miette::{Result, WrapErr, miette};
use serde::Deserialize;
use templater::{Template, function::FunctionRegistry, value::Value};
use walkdir::WalkDir;

use crate::data::DataFile;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum DeployType {
    Symlink,
    Copy,
    Template,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "DeployModeRepr")]
pub struct DeployMode(u32);

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
        Ok(Self(u32::from_str_radix(&value, 8).unwrap()))
    }
}

impl TryFrom<u32> for DeployMode {
    type Error = miette::Report;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        if (0..=0o777).contains(&value) {
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

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedPortal {
    source: PathBuf,
    target: PathBuf,
}

// TODO
struct NoFunctions;
impl FunctionRegistry for NoFunctions {
    fn call(
        &self,
        name: &str,
        _: &[Value],
    ) -> std::result::Result<Value, templater::error::RegistryError> {
        Err(templater::error::RegistryError::Undefined { name: name.into() })
    }
}

pub fn read(source: &Path, target_override: Option<PathBuf>) -> Result<DesiredDeployment> {
    if !source.is_dir() {
        return Err(miette!(
            "source directory `{}` does not exist",
            source.display()
        ));
    }
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
    })
}

fn render_config(path: &Path, context: &HashMap<String, Value>) -> Result<String> {
    let bytes = fs::read(path)
        .map_err(|error| miette!(error))
        .wrap_err_with(|| format!("cannot read `{}`", path.display()))?;
    let template = Template::from_bytes(bytes);
    let mut output = Vec::new();
    let result = template.render(&mut output, context, &NoFunctions);
    template
        .report(result)
        .map_err(|error| miette!(error))
        .wrap_err_with(|| format!("cannot render `{}`", path.display()))?;
    String::from_utf8(output)
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
    match builder.add(path) {
        None => {}
        Some(error) => return Err(miette!(error)),
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

fn resolve_portals(
    source: &Path,
    portals: &BTreeMap<String, String>,
) -> Result<Vec<ResolvedPortal>> {
    let mut result = Vec::new();
    for (key, value) in portals {
        validate_relative(key, "portal source")?;
        validate_relative(value, "portal target")?;
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
            if push_deployable(&mut result, &path, Path::new(value)).is_ok() {
                continue;
            } else if is_directory(&path) {
                for item in WalkDir::new(&path).follow_links(false) {
                    let item = item.map_err(|error| miette!(error))?;
                    if item.path() == path || is_directory(item.path()) {
                        continue;
                    }
                    let relative = item.path().strip_prefix(&path).expect("descendant");
                    push_deployable(&mut result, item.path(), &Path::new(value).join(relative))?;
                }
            } else {
                return Err(miette!(
                    "portal source `{}` is not a regular file, symlink to a regular file or directory",
                    path.display()
                ));
            }
            continue;
        }

        let strip = stripping_prefix(key);
        let pattern = Pattern::new(key)
            .map_err(|error| miette!("invalid portal pattern `{key}` because {error}"))?;
        for item in WalkDir::new(source).follow_links(false) {
            let item = item.map_err(|error| miette!(error))?;
            let path = item.path();
            if path == source || is_directory(path) {
                continue;
            }
            let relative = path
                .strip_prefix(source)
                .expect("walkdir path below source");
            if pattern.matches_path(relative) {
                let remainder = relative.strip_prefix(&strip).unwrap_or(relative);
                push_deployable(&mut result, path, &PathBuf::from(value).join(remainder))?;
            }
        }
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

fn stripping_prefix(pattern: &str) -> PathBuf {
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
                let (name, child) = children.iter().next().expect("dir node has children");
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
        let pattern = Pattern::new(pattern.strip_prefix("./").unwrap_or(pattern))
            .map_err(|error| miette!("invalid rule pattern `{pattern}` because {error}"))?;
        compiled.insert(pattern, rule.clone());
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
        if pattern.matches_path(&entry.target) {
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

fn is_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_dir())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use test_case::test_case;

    use super::*;

    macro_rules! portals {
        ($($key:expr => $value:expr),* $(,)?) => {
            BTreeMap::from([$(($key.to_string(), $value.to_string())),*])
        };
    }

    macro_rules! resolve {
        ($source:expr, $target:expr) => {
            ResolvedPortal {
                source: PathBuf::from($source),
                target: PathBuf::from($target),
            }
        };
    }

    macro_rules! rules {
        ($($pattern:expr => $rule:expr),* $(,)?) => {
            indexmap::IndexMap::from([$(($pattern.to_string(), $rule)),*])
        };
    }

    macro_rules! rule {
        ($type:expr, $mode:expr) => {
            RuleConfig {
                deploy_type: $type,
                mode: $mode,
            }
        };
    }

    macro_rules! deploy {
        ($source:expr, $target:expr, $type:expr, $mode:expr) => {
            DeploymentEntry {
                source_path: PathBuf::from($source),
                target_path: PathBuf::from($target),
                deploy_type: $type,
                mode: $mode,
            }
        };
    }

    fn mode(octal: &str) -> DeployMode {
        DeployMode::try_from(octal.to_string()).expect("valid octal mode")
    }

    #[test_case(
        |t| fs::write(t.join("file.txt"), "hello").unwrap(),
        portals! { "file.txt" => "out.txt" } => vec![resolve!("file.txt", "out.txt")];
        "literal_file_portal"
    )]
    #[test_case(
        |t| fs::write(t.join("file.txt"), "hello").unwrap(),
        portals! { "./file.txt" => "./out.txt" } => vec![resolve!("file.txt", "out.txt")];
        "literal_file_dot_prefix"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("file.txt"), "").unwrap();
            std::os::unix::fs::symlink(t.join("file.txt"), t.join("link")).unwrap();
        },
        portals! { "link" => "out" } => vec![resolve!("link", "out")];
        "literal_symlink_to_file"
    )]
    #[test_case(
        |t| fs::create_dir(t.join("empty")).unwrap(),
        portals! { "empty" => "dst" } => Vec::<ResolvedPortal>::new();
        "literal_empty_directory"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("dir/sub")).unwrap();
            fs::write(t.join("dir/a.txt"), "").unwrap();
            fs::write(t.join("dir/sub/b.txt"), "").unwrap();
        },
        portals! { "dir" => "dst" } => vec![
            resolve!("dir/a.txt", "dst/a.txt"),
            resolve!("dir/sub/b.txt", "dst/sub/b.txt")
        ];
        "literal_directory_recurses"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("dir")).unwrap();
            fs::write(t.join("a.txt"), "").unwrap();
        },
        portals! { "*" => "dst" } => vec![resolve!("a.txt", "dst/a.txt")];
        "wildcard_skips_directories"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("a.txt"), "").unwrap();
            fs::write(t.join("b.txt"), "").unwrap();
            fs::write(t.join("other.md"), "").unwrap();
        },
        portals! { "*.txt" => "dst" } => vec![
            resolve!("a.txt", "dst/a.txt"),
            resolve!("b.txt", "dst/b.txt")
        ];
        "wildcard_top_level_files"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("configs/sub")).unwrap();
            fs::write(t.join("configs/a.txt"), "").unwrap();
            fs::write(t.join("configs/sub/b.txt"), "").unwrap();
        },
        portals! { "configs/**" => "dst" } => vec![
            resolve!("configs/a.txt", "dst/a.txt"),
            resolve!("configs/sub/b.txt", "dst/sub/b.txt")
        ];
        "wildcard_prefix_stripped"
    )]
    #[test_case(
        |_| {},
        portals! { "/abs" => "dst" } => panics "invalid portal source path `/abs`";
        "absolute_source_rejected"
    )]
    #[test_case(
        |t| fs::write(t.join("file.txt"), "hello").unwrap(),
        portals! { "file.txt" => "/abs" } => panics "invalid portal target path `/abs`";
        "absolute_target_rejected"
    )]
    #[test_case(
        |_| {},
        portals! { "../x" => "dst" } => panics "invalid portal source path `../x`";
        "parent_component_in_source_rejected"
    )]
    #[test_case(
        |t| fs::write(t.join("file.txt"), "hello").unwrap(),
        portals! { "file.txt" => "../dst" } => panics "invalid portal target path `../dst`";
        "parent_component_in_target_rejected"
    )]
    #[test_case(
        |_| {},
        portals! { "" => "dst" } => panics "invalid portal source path";
        "empty_source_rejected"
    )]
    #[test_case(
        |t| fs::write(t.join("file.txt"), "hello").unwrap(),
        portals! { "file.txt" => "" } => panics "invalid portal target path";
        "empty_target_rejected"
    )]
    #[test_case(
        |_| {},
        portals! { "missing" => "dst" } => panics "literal portal source `missing` does not exist";
        "literal_source_missing"
    )]
    #[test_case(
        |t| {
            std::os::unix::fs::symlink(t.join("missing"), t.join("dangling")).unwrap();
        },
        portals! { "dangling" => "dst" } => panics "is not a regular file, symlink to a regular file or directory";
        "broken_symlink_source_rejected"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("dir")).unwrap();
            std::os::unix::fs::symlink(t.join("missing"), t.join("dir/dangling")).unwrap();
        },
        portals! { "dir" => "dst" } => panics "is not a regular file or symlink to a regular file";
        "directory_containing_broken_symlink_rejected"
    )]
    #[test_case(
        |_| {},
        portals! { "[" => "dst" } => panics "invalid portal pattern `[`";
        "invalid_portal_pattern"
    )]
    #[test_case(
        |t| fs::write(t.join("file.txt"), "hello").unwrap(),
        portals! { "file.txt" => "*.txt" } => panics "portal target `*.txt` cannot contain glob syntax";
        "target_with_glob_rejected"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("real")).unwrap();
            std::os::unix::fs::symlink(t.join("real"), t.join("dirlink")).unwrap();
        },
        portals! { "*" => "dst" } => panics "is not a regular file or symlink to a regular file";
        "wildcard_matching_symlink_to_dir_rejected"
    )]
    #[test_case(
        |t| std::os::unix::fs::symlink(t.join("missing"), t.join("dangling")).unwrap(),
        portals! { "*" => "dst" } => panics "is not a regular file or symlink to a regular file";
        "wildcard_matching_broken_symlink_rejected"
    )]
    fn resolve_portals_test<F: Fn(&Path)>(
        setup: F,
        portals: BTreeMap<String, String>,
    ) -> Vec<ResolvedPortal> {
        let tmp = tempdir().expect("cannot create temp dir");
        setup(tmp.path());
        let mut result = resolve_portals(tmp.path(), &portals).unwrap_or_else(|e| panic!("{e}"));
        result.sort_by(|a, b| (&a.source, &a.target).cmp(&(&b.source, &b.target)));
        for entry in &mut result {
            entry.source = entry
                .source
                .strip_prefix(tmp.path())
                .expect("source below temp dir")
                .to_path_buf();
        }
        result
    }

    #[test_case(vec![] => (); "empty")]
    #[test_case(
        vec![resolve!("s1.txt", "dst.txt")] => ();
        "single_file"
    )]
    #[test_case(
        vec![resolve!("s1", "a"), resolve!("s2", "b"), resolve!("s3", "c")] => ();
        "distinct_files"
    )]
    #[test_case(
        vec![resolve!("s1", "a/b"), resolve!("s2", "a/c"), resolve!("s3", "d/e/f")] => ();
        "nested_distinct_paths"
    )]
    #[test_case(
        vec![resolve!("s1", "a"), resolve!("s2", "a")] => panics "collision at `a` between `s1` and `s2`";
        "duplicate_target_collision"
    )]
    #[test_case(
        vec![resolve!("s1", "a/x"), resolve!("s2", "a/x")] => panics "collision at `a/x` between `s1` and `s2`";
        "duplicate_nested_target_collision"
    )]
    #[test_case(
        vec![resolve!("s1", "a"), resolve!("s2", "a/b")] => panics "structural conflict between `a` and `a/b`";
        "file_then_descendant_conflict"
    )]
    #[test_case(
        vec![resolve!("s1", "a/b"), resolve!("s2", "a/b/c")] => panics "structural conflict between `a/b` and `a/b/c`";
        "deep_file_then_descendant_conflict"
    )]
    #[test_case(
        vec![resolve!("s1", "a/b"), resolve!("s2", "a")] => panics "structural conflict between `a` and `a/b/`";
        "descendant_then_file_conflict"
    )]
    #[test_case(
        vec![resolve!("s1", "a/b/c"), resolve!("s2", "a/b")] => panics "structural conflict between `a/b` and `a/b/c/`";
        "deep_descendant_then_file_conflict"
    )]
    fn validate_targets_test(entries: Vec<ResolvedPortal>) {
        validate_targets(&entries).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test_case(
        resolve!("a.txt", "a.txt"),
        rules! {} => deploy!("a.txt", "/a.txt", DeployType::Symlink, None);
        "no_rules_default_symlink"
    )]
    #[test_case(
        resolve!("a.txt", "a.txt"),
        rules! { "*.md" => rule!(Some(DeployType::Copy), None) } => deploy!("a.txt", "/a.txt", DeployType::Symlink, None);
        "rule_does_not_match"
    )]
    #[test_case(
        resolve!("a.txt", "a.txt"),
        rules! { "*.txt" => rule!(Some(DeployType::Copy), None) } => deploy!("a.txt", "/a.txt", DeployType::Copy, None);
        "copy_rule_applies"
    )]
    #[test_case(
        resolve!("a.toml", "a.toml"),
        rules! { "*.toml" => rule!(Some(DeployType::Template), None) } => deploy!("a.toml", "/a.toml", DeployType::Template, None);
        "template_rule_applies"
    )]
    #[test_case(
        resolve!("a.txt", "a.txt"),
        rules! { "./*.txt" => rule!(Some(DeployType::Copy), None) } => deploy!("a.txt", "/a.txt", DeployType::Copy, None);
        "dot_prefix_rule_pattern"
    )]
    #[test_case(
        resolve!("src", "configs/sub/file"),
        rules! { "configs/**" => rule!(Some(DeployType::Copy), None) } => deploy!("src", "/configs/sub/file", DeployType::Copy, None);
        "nested_glob_matches"
    )]
    #[test_case(
        resolve!("a.sh", "a.sh"),
        rules! { "*.sh" => rule!(Some(DeployType::Copy), Some(mode("755"))) } => deploy!("a.sh", "/a.sh", DeployType::Copy, Some(mode("755")));
        "mode_with_copy"
    )]
    #[test_case(
        resolve!("a.txt", "a.txt"),
        rules! {
            "*.txt" => rule!(Some(DeployType::Copy), None),
            "a.*" => rule!(Some(DeployType::Template), None)
        } => deploy!("a.txt", "/a.txt", DeployType::Template, None);
        "last_matching_rule_wins"
    )]
    #[test_case(
        resolve!("a.txt", "a.txt"),
        rules! {
            "*.txt" => rule!(Some(DeployType::Copy), None),
            "a.txt" => rule!(None, Some(mode("644")))
        } => deploy!("a.txt", "/a.txt", DeployType::Copy, Some(mode("644")));
        "fields_merge_across_rules"
    )]
    #[test_case(
        resolve!("a.txt", "a.txt"),
        rules! { "/abs" => rule!(None, None) } => panics "invalid rule path `/abs`";
        "absolute_rule_pattern_rejected"
    )]
    #[test_case(
        resolve!("a.txt", "a.txt"),
        rules! { ".." => rule!(None, None) } => panics "invalid rule path `..`";
        "parent_rule_pattern_rejected"
    )]
    #[test_case(
        resolve!("a.txt", "a.txt"),
        rules! { "" => rule!(None, None) } => panics "invalid rule path";
        "empty_rule_pattern_rejected"
    )]
    #[test_case(
        resolve!("a.txt", "a.txt"),
        rules! { "[" => rule!(None, None) } => panics "invalid rule pattern `[`";
        "invalid_glob_pattern_rejected"
    )]
    #[test_case(
        resolve!("a.txt", "a.txt"),
        rules! { "*.txt" => rule!(None, Some(mode("755"))) } => panics "conflicting rules for `a.txt`: `mode` is set but the effective `type` is `symlink`";
        "mode_conflicts_with_default_symlink"
    )]
    #[test_case(
        resolve!("a.txt", "a.txt"),
        rules! { "*.txt" => rule!(Some(DeployType::Symlink), Some(mode("755"))) } => panics "conflicting rules for `a.txt`: `mode` is set but the effective `type` is `symlink`";
        "mode_conflicts_with_explicit_symlink"
    )]
    fn apply_rules_test(
        entry: ResolvedPortal,
        rules: indexmap::IndexMap<String, RuleConfig>,
    ) -> DeploymentEntry {
        let compiled = compile_rules(&rules).unwrap_or_else(|e| panic!("{e}"));
        apply_rules(entry, &compiled, Path::new("/")).unwrap_or_else(|e| panic!("{e}"))
    }
}
