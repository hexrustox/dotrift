use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsString,
    fs,
    os::unix::fs::MetadataExt,
    path::{Component, Path, PathBuf},
};

use glob::Pattern;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use miette::{Result, WrapErr, miette};
use serde::Deserialize;
use templater::value::Value;

use crate::data::DataFile;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DeployType {
    Symlink,
    Copy,
    Template,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "DeployModeRepr")]
pub(crate) struct DeployMode(u32);

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
pub(crate) struct DeploymentEntry {
    pub source_path: PathBuf,
    pub target_path: PathBuf,
    pub deploy_type: DeployType,
    pub mode: Option<DeployMode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DesiredDeployment {
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

pub(crate) fn read(source: &Path, target_override: Option<PathBuf>) -> Result<DesiredDeployment> {
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
            if !pattern.matches_path(relative) {
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
