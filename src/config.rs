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

#[derive(Debug, Clone)]
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
        let wildcard = contains_wildcard(key);
        if !wildcard {
            let path = source.join(key);
            if !path.exists() && fs::symlink_metadata(&path).is_err() {
                return Err(miette!("literal portal source `{key}` does not exist"));
            }
            add_source(&mut result, &path, Path::new(value))?;
            continue;
        }
        let strip = stripping_prefix(key);
        for item in WalkDir::new(source).follow_links(false) {
            let item = item.map_err(|error| miette!(error))?;
            let path = item.path();
            if path == source || is_directory(path) {
                continue;
            }
            let relative = path
                .strip_prefix(source)
                .expect("walkdir path below source");
            let pattern = Pattern::new(key)
                .map_err(|error| miette!("invalid portal pattern `{key}` because {error}"))?;
            if pattern.matches_path(relative) {
                if !is_deployable_file(path) {
                    return Err(miette!(
                        "source path `{}` is not a regular file or symlink to a regular file",
                        relative.display()
                    ));
                }
                let remainder = relative.strip_prefix(&strip).unwrap_or(relative);
                result.push(ResolvedPortal {
                    source: path.to_path_buf(),
                    target: PathBuf::from(value).join(remainder),
                });
            }
        }
    }
    Ok(result)
}

fn add_source(result: &mut Vec<ResolvedPortal>, source: &Path, target: &Path) -> Result<()> {
    if is_deployable_file(source) {
        result.push(ResolvedPortal {
            source: source.to_path_buf(),
            target: target.to_path_buf(),
        });
        return Ok(());
    }
    if is_directory(source) {
        for item in WalkDir::new(source).follow_links(false) {
            let item = item.map_err(|error| miette!(error))?;
            if item.path() == source || is_directory(item.path()) {
                continue;
            }
            if !is_deployable_file(item.path()) {
                return Err(miette!(
                    "source path `{}` is not a regular file or symlink to a regular file",
                    item.path().display()
                ));
            }
            let relative = item.path().strip_prefix(source).expect("descendant");
            result.push(ResolvedPortal {
                source: item.path().to_path_buf(),
                target: target.join(relative),
            });
        }
        return Ok(());
    }
    Err(miette!(
        "portal source `{}` is not a regular file or directory",
        source.display()
    ))
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

fn is_deployable_file(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.is_file())
}

fn is_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_dir())
}
