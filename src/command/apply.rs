use std::collections::HashMap;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, Result, eyre};
use glob::glob;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use normalize_path::NormalizePath;
use walkdir::WalkDir;

use crate::cli::ApplyFlags;
use crate::config::{Config, DeployType, FileMode};

pub struct FileIntent {
    pub source: PathBuf,
    pub action_type: DeployType,
    pub mode: Option<FileMode>,
}

pub fn run(
    source_dir: PathBuf,
    target_override: Option<PathBuf>,
    _flags: ApplyFlags,
) -> Result<()> {
    let source_normalized = source_dir.normalize();

    let config = Config::read(source_dir.clone())?;

    let target_dir = resolve_target(target_override, &config)?;
    let target_normalized = target_dir.normalize();

    validate_paths(&source_normalized, &target_normalized)?;

    let ignore_matcher = build_ignore(&config.ignore, &target_normalized)?;

    let _intents = resolve_portals(
        &source_normalized,
        &target_normalized,
        &config.portal,
        &ignore_matcher,
    )?;

    Ok(())
}

fn resolve_target(target_override: Option<PathBuf>, config: &Config) -> Result<PathBuf> {
    validate_absolute(
        &target_override
            .or(config.target_dir.clone())
            .or(dirs::home_dir())
            .ok_or_else(|| eyre!("Cannot determine target directory."))?,
    )
}

fn validate_absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Err(eyre!(
            "Target directory must be an absolute path: `{}`.",
            path.display()
        ))
    }
}

fn validate_paths(source_dir: &Path, target_dir: &Path) -> Result<()> {
    if source_dir == target_dir {
        return Err(eyre!("Source directory cannot equal target directory."));
    }

    if target_dir.starts_with(source_dir) {
        return Err(eyre!("Target directory cannot be inside source directory."));
    }

    Ok(())
}

fn build_ignore(patterns: &[String], target_dir: &Path) -> Result<Gitignore> {
    let mut builder = GitignoreBuilder::new(target_dir);
    for pattern in patterns {
        builder
            .add_line(None, pattern)
            .map_err(|e| eyre!("Invalid ignore pattern `{}`: {}.", pattern, e))?;
    }
    builder
        .build()
        .map_err(|e| eyre!("Failed to build ignore matcher: {}.", e))
}

fn is_glob(pattern: &str) -> bool {
    pattern.contains(['*', '?', '['])
}

fn stripping_prefix(glob_pattern: &str) -> String {
    let mut prefix = String::new();
    for component in glob_pattern.split('/') {
        if component.contains(['*', '?', '[']) {
            break;
        }
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(component);
    }
    if !prefix.is_empty() {
        prefix.push('/');
    }
    prefix
}

fn resolve_portals(
    source_dir: &Path,
    target_dir: &Path,
    portals: &HashMap<String, PathBuf>,
    ignore_matcher: &Gitignore,
) -> Result<HashMap<PathBuf, FileIntent>> {
    let mut intents: HashMap<PathBuf, FileIntent> = HashMap::new();

    for (pattern, target_rel) in portals {
        let pattern_normalized = Path::new(pattern).normalize();
        let pattern_str = pattern_normalized.to_string_lossy();
        let target_rel_normalized = target_rel.normalize();

        if is_glob(&pattern_str) {
            resolve_glob_portal(
                source_dir,
                target_dir,
                &pattern_str,
                &target_rel_normalized,
                ignore_matcher,
                &mut intents,
            )?;
        } else {
            resolve_literal_portal(
                source_dir,
                target_dir,
                &pattern_normalized,
                &target_rel_normalized,
                ignore_matcher,
                &mut intents,
            )?;
        }
    }

    Ok(intents)
}

fn resolve_glob_portal(
    source_dir: &Path,
    target_dir: &Path,
    pattern: &str,
    target_rel: &Path,
    ignore_matcher: &Gitignore,
    intents: &mut HashMap<PathBuf, FileIntent>,
) -> Result<()> {
    let prefix = stripping_prefix(pattern);
    let full_pattern = source_dir.join(pattern);
    let full_pattern_str = full_pattern.to_string_lossy();

    for entry in glob(&full_pattern_str).wrap_err("Invalid glob pattern.")? {
        let source_path = entry.wrap_err("Error reading glob match.")?;

        let source_rel = source_path
            .strip_prefix(source_dir)
            .wrap_err("Source path escapes source directory.")?;

        let stripped = if prefix.is_empty() {
            source_rel.to_path_buf()
        } else {
            source_rel
                .strip_prefix(&prefix)
                .unwrap_or(source_rel)
                .to_path_buf()
        };

        let target_path = target_dir.join(target_rel).join(stripped).normalize();

        check_traversal(&target_path, target_dir)?;

        let target_rel_for_ignore = target_path.strip_prefix(target_dir).unwrap();
        if is_ignored(ignore_matcher, target_rel_for_ignore) {
            continue;
        }

        insert_intent(intents, target_path, source_path)?;
    }

    Ok(())
}

fn resolve_literal_portal(
    source_dir: &Path,
    target_dir: &Path,
    pattern: &Path,
    target_rel: &Path,
    ignore_matcher: &Gitignore,
    intents: &mut HashMap<PathBuf, FileIntent>,
) -> Result<()> {
    let source_path = source_dir.join(pattern).normalize();

    check_traversal(&source_path, source_dir)?;

    if !source_path.exists() {
        return Err(eyre!(
            "Source path does not exist: `{}`.",
            source_path.display()
        ));
    }

    if source_path.is_file() {
        let target_path = target_dir.join(target_rel).normalize();

        check_traversal(&target_path, target_dir)?;

        let target_rel_for_ignore = target_path.strip_prefix(target_dir).unwrap();
        if is_ignored(ignore_matcher, target_rel_for_ignore) {
            return Ok(());
        }

        insert_intent(intents, target_path, source_path)?;
    } else {
        for entry in WalkDir::new(&source_path)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let file_source = entry.path().normalize();

            let rel_to_pattern = file_source
                .strip_prefix(&source_path)
                .wrap_err("Source file escapes pattern directory.")?;

            let target_path = target_dir.join(target_rel).join(rel_to_pattern).normalize();

            check_traversal(&target_path, target_dir)?;

            let target_rel_for_ignore = target_path.strip_prefix(target_dir).unwrap();
            if is_ignored(ignore_matcher, target_rel_for_ignore) {
                continue;
            }

            insert_intent(intents, target_path, file_source)?;
        }
    }

    Ok(())
}

fn check_traversal(path: &Path, base: &Path) -> Result<()> {
    if !path.starts_with(base) {
        return Err(eyre!("Path escapes base directory: `{}`.", path.display()));
    }
    Ok(())
}

fn insert_intent(
    intents: &mut HashMap<PathBuf, FileIntent>,
    target_path: PathBuf,
    source_path: PathBuf,
) -> Result<()> {
    if let Some(existing) = intents.insert(
        target_path.clone(),
        FileIntent {
            source: source_path.clone(),
            action_type: DeployType::default(),
            mode: None,
        },
    ) {
        return Err(eyre!(
            "Target path collision at `{}`.\n  Source 1: `{}`\n  Source 2: `{}`",
            target_path.display(),
            existing.source.display(),
            source_path.display()
        ));
    }
    Ok(())
}

fn is_ignored(matcher: &Gitignore, path: &Path) -> bool {
    matcher.matched(path, false).is_ignore()
}
