use std::{
    collections::{BTreeMap, HashMap},
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeployType {
    Symlink,
    Copy,
    Template,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentEntry {
    pub source_path: PathBuf,
    pub target_path: PathBuf,
    pub deploy_type: DeployType,
    pub mode: Option<u32>,
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleConfig {
    #[serde(rename = "type")]
    deploy_type: Option<String>,
    mode: Option<String>,
}

#[derive(Debug, Clone)]
struct ResolvedPortal {
    source: PathBuf,
    target: PathBuf,
}

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

pub fn read(source: &Path, target_override: Option<&Path>) -> Result<DesiredDeployment> {
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
    let mut document: toml::Value = toml::from_str(&rendered)
        .map_err(|error| miette!(error))
        .wrap_err_with(|| format!("cannot parse `{}`", config_path.display()))?;
    if target_override.is_some() {
        document
            .as_table_mut()
            .expect("TOML root is a table")
            .remove("target-directory");
    }
    let config: FileConfig = document
        .try_into()
        .map_err(|error| miette!(error))
        .wrap_err_with(|| format!("cannot parse `{}`", config_path.display()))?;
    let target = target_override
        .map(Path::to_path_buf)
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
    validate_collisions(&portals)?;
    validate_structural_conflicts(&portals)?;
    validate_rules(&config.rule)?;
    let entries = portals
        .into_iter()
        .map(|entry| apply_rules(entry, &config.rule, &target))
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
    if let Ok(text) = fs::read_to_string(&path) {
        for line in text.lines() {
            builder
                .add_line(Some(path.clone()), line)
                .map_err(|error| miette!(error))?;
        }
    } else if path.exists() {
        return Err(miette!("cannot read `{}`", path.display()));
    }
    builder.build().map_err(|error| miette!(error))
}

fn resolve_portals(
    source: &Path,
    portals: &BTreeMap<String, String>,
) -> Result<Vec<ResolvedPortal>> {
    let mut result = Vec::new();
    for (key, value) in portals {
        validate_relative(key, "portal source")?;
        validate_relative(value, "portal target")?;
        if value
            .bytes()
            .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b'{' | b'}'))
        {
            return Err(miette!(
                "portal target `{value}` cannot contain glob syntax"
            ));
        }
        let key = key.strip_prefix("./").unwrap_or(key);
        let value = value.strip_prefix("./").unwrap_or(value);
        let pattern = Pattern::new(key)
            .map_err(|error| miette!("invalid portal pattern `{key}` because {error}"))?;
        let wildcard = key.bytes().any(|byte| matches!(byte, b'*' | b'?' | b'['));
        if !wildcard {
            let path = source.join(key.trim_start_matches("./"));
            if !path.exists() && fs::symlink_metadata(&path).is_err() {
                return Err(miette!("literal portal source `{key}` does not exist"));
            }
            add_source(&mut result, source, &path, Path::new(value))?;
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

fn add_source(
    result: &mut Vec<ResolvedPortal>,
    _source_root: &Path,
    source: &Path,
    target: &Path,
) -> Result<()> {
    if fs::symlink_metadata(source).is_ok_and(|metadata| metadata.file_type().is_symlink())
        && !is_deployable_file(source)
    {
        return Err(miette!(
            "portal source `{}` is not a regular file or directory",
            source.display()
        ));
    }
    if is_deployable_file(source) {
        result.push(ResolvedPortal {
            source: source.to_path_buf(),
            target: target.to_path_buf(),
        });
        return Ok(());
    }
    if source.is_dir() {
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

fn is_deployable_file(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.is_file())
}

fn is_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_dir())
}

fn stripping_prefix(pattern: &str) -> PathBuf {
    let mut prefix = PathBuf::new();
    for component in Path::new(pattern).components() {
        let value = component.as_os_str().to_string_lossy();
        if value.bytes().any(|byte| matches!(byte, b'*' | b'?' | b'[')) {
            break;
        }
        prefix.push(value.as_ref());
    }
    prefix
}

fn validate_relative(value: &str, what: &str) -> Result<()> {
    if value.is_empty()
        || Path::new(value).is_absolute()
        || value.contains('{')
        || value.contains('}')
    {
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

fn validate_collisions(entries: &[ResolvedPortal]) -> Result<()> {
    let mut seen: BTreeMap<&Path, Vec<&Path>> = BTreeMap::new();
    for entry in entries {
        let sources = seen.entry(&entry.target).or_default();
        sources.push(&entry.source);
    }
    if let Some((target, sources)) = seen.iter().find(|(_, sources)| sources.len() > 1) {
        let contributors = sources
            .iter()
            .map(|source| format!("`{}`", source.display()))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(miette!(
            "collision at `{}` from source paths {}",
            target.display(),
            contributors
        ));
    }
    Ok(())
}

fn apply_rules(
    entry: ResolvedPortal,
    rules: &indexmap::IndexMap<String, RuleConfig>,
    target_root: &Path,
) -> Result<DeploymentEntry> {
    let mut deploy_type = DeployType::Symlink;
    let mut mode = None;
    for (pattern, rule) in rules {
        validate_relative(pattern, "rule")?;
        let pattern = pattern.strip_prefix("./").unwrap_or(pattern);
        if Pattern::new(pattern)
            .map_err(|error| miette!("invalid rule pattern `{pattern}` because {error}"))?
            .matches_path(&entry.target)
        {
            if let Some(value) = &rule.deploy_type {
                deploy_type = match value.as_str() {
                    "symlink" => DeployType::Symlink,
                    "copy" => DeployType::Copy,
                    "template" => DeployType::Template,
                    _ => return Err(miette!("invalid deploy type `{value}`")),
                };
            }
            if let Some(value) = &rule.mode {
                if value.len() != 3 || !value.bytes().all(|byte| (b'0'..=b'7').contains(&byte)) {
                    return Err(miette!("invalid mode `{value}`"));
                }
                mode = Some(u32::from_str_radix(value, 8).unwrap());
            }
        }
    }
    if mode.is_some() && deploy_type == DeployType::Symlink {
        return Err(miette!("mode cannot be used with symlink deployment"));
    }
    Ok(DeploymentEntry {
        source_path: entry.source,
        target_path: target_root.join(entry.target),
        deploy_type,
        mode,
    })
}

fn validate_structural_conflicts<T: TargetPath>(entries: &[T]) -> Result<()> {
    for (index, left) in entries.iter().enumerate() {
        for right in entries.iter().skip(index + 1) {
            if is_ancestor(left.target(), right.target())
                || is_ancestor(right.target(), left.target())
            {
                return Err(miette!(
                    "structural conflict between `{}` and `{}`",
                    left.target().display(),
                    right.target().display()
                ));
            }
        }
    }
    Ok(())
}

fn is_ancestor(left: &Path, right: &Path) -> bool {
    left == Path::new(".") || right.starts_with(left)
}

trait TargetPath {
    fn target(&self) -> &Path;
}

impl TargetPath for ResolvedPortal {
    fn target(&self) -> &Path {
        &self.target
    }
}

impl TargetPath for DeploymentEntry {
    fn target(&self) -> &Path {
        &self.target_path
    }
}

fn validate_rules(rules: &indexmap::IndexMap<String, RuleConfig>) -> Result<()> {
    for (pattern, rule) in rules {
        validate_relative(pattern, "rule")?;
        Pattern::new(pattern.strip_prefix("./").unwrap_or(pattern))
            .map_err(|error| miette!("invalid rule pattern `{pattern}` because {error}"))?;
        if let Some(value) = &rule.deploy_type {
            if !matches!(value.as_str(), "symlink" | "copy" | "template") {
                return Err(miette!("invalid deploy type `{value}`"));
            }
        }
        if let Some(value) = &rule.mode {
            if value.len() != 3 || !value.bytes().all(|byte| (b'0'..=b'7').contains(&byte)) {
                return Err(miette!("invalid mode `{value}`"));
            }
        }
        if rule.mode.is_some() && rule.deploy_type.as_deref() == Some("symlink") {
            return Err(miette!("`mode` cannot be used with `type` `symlink`"));
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn resolves_literal_directory_and_rules_in_order() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fs::create_dir_all(source.join("config/secrets")).unwrap();
        fs::write(source.join("config/a"), "a").unwrap();
        fs::write(source.join("config/secrets/x"), "x").unwrap();
        fs::write(source.join("dotrift.toml"), "[portal]\n\"config\" = \"files\"\n[rule]\n\"files/**\" = { type=\"copy\" }\n\"files/secrets/**\" = { mode=\"600\" }").unwrap();
        let result = read(&source, Some(&dir.path().join("target"))).unwrap();
        assert_eq!(result.entries.len(), 2);
        assert_eq!(
            result
                .entries
                .iter()
                .find(|entry| entry.target_path.ends_with("files/secrets/x"))
                .unwrap()
                .mode,
            Some(0o600)
        );
    }

    #[test]
    fn ignores_then_reincludes_entries() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fs::create_dir_all(source.join("x")).unwrap();
        fs::write(source.join("x/a"), "a").unwrap();
        fs::write(source.join("dotrift.toml"), "[portal]\n\"x/**\" = \".\"").unwrap();
        fs::write(source.join(".dotriftignore"), "a\n!a\n").unwrap();
        assert_eq!(
            read(&source, Some(&dir.path().join("target")))
                .unwrap()
                .entries
                .len(),
            1
        );
    }

    #[test]
    fn implicitly_ignores_control_files() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("dotrift.toml"), "[portal]\n\"**\" = \".\"").unwrap();
        fs::write(source.join("dotrift_data.toml"), "").unwrap();
        fs::write(source.join(".dotriftignore"), "").unwrap();
        fs::write(source.join("file"), "content").unwrap();
        let result = read(&source, Some(&dir.path().join("target"))).unwrap();
        assert_eq!(result.entries.len(), 1);
        assert!(result.entries[0].target_path.ends_with("file"));
    }

    #[test]
    fn rejects_collisions_before_rules() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fs::create_dir_all(source.join("one")).unwrap();
        fs::write(source.join("one/a"), "a").unwrap();
        fs::write(source.join("one/b"), "b").unwrap();
        fs::write(
            source.join("dotrift.toml"),
            "[portal]\n\"one/a\" = \"same\"\n\"one/b\" = \"same\"",
        )
        .unwrap();
        assert!(read(&source, Some(&dir.path().join("target"))).is_err());
    }
}
