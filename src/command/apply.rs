use std::collections::HashMap;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, Result, eyre};
use glob::{MatchOptions, glob_with};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use normalize_path::NormalizePath;
use walkdir::WalkDir;

use crate::cli::ApplyFlags;
use crate::config::{Config, DeployType, FileMode, Rules};

#[derive(Default, Debug, PartialEq)]
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

    let mut intents = resolve_portals(
        &source_normalized,
        &target_normalized,
        &config.portal,
        &ignore_matcher,
    )?;

    apply_rules(&mut intents, &config.rule)?;

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
    builder.add_line(None, "dotrift.toml").unwrap();
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
        if is_glob(component) {
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

    for entry in glob_with(
        &full_pattern_str,
        MatchOptions {
            case_sensitive: true,
            require_literal_separator: true,
            require_literal_leading_dot: false,
        },
    )
    .wrap_err("Invalid glob pattern.")?
    {
        let source_path = entry.wrap_err("Error reading glob match.")?;

        let source_rel = source_path.strip_prefix(source_dir).unwrap();

        let stripped = if prefix.is_empty() {
            source_rel.to_path_buf()
        } else {
            source_rel
                .strip_prefix(&prefix)
                .unwrap_or(source_rel)
                .to_path_buf()
        };

        let target_path = target_dir.join(target_rel).join(stripped);

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
    let source_path = source_dir.join(pattern);

    if !source_path.exists() {
        return Err(eyre!(
            "Source path does not exist: `{}`.",
            source_path.display()
        ));
    }

    if source_path.is_file() {
        let target_path = target_dir.join(target_rel);

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
            let file_source = entry.path().to_path_buf();

            let rel_to_pattern = file_source.strip_prefix(&source_path).unwrap();

            let target_path = target_dir.join(target_rel).join(rel_to_pattern);

            let target_rel_for_ignore = target_path.strip_prefix(target_dir).unwrap();
            if is_ignored(ignore_matcher, target_rel_for_ignore) {
                continue;
            }

            insert_intent(intents, target_path, file_source)?;
        }
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

fn apply_rules(intents: &mut HashMap<PathBuf, FileIntent>, rules: &Rules) -> Result<()> {
    for (pattern, rule) in rules {
        let pattern = glob::Pattern::new(pattern).wrap_err("Invalid glob pattern.")?;
        for (path, intent) in intents.iter_mut() {
            if !pattern.matches(&path.to_string_lossy()) {
                continue;
            }
            if let Some(rule_type) = &rule.r#type {
                intent.action_type = *rule_type;
            }
            if let Some(rule_mode) = &rule.mode {
                intent.mode = Some(*rule_mode);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;
    use test_case::test_case;

    macro_rules! mk_intent {
        ($(($s:literal, $t:literal)),*) => {
            HashMap::from_iter([$((PathBuf::from("target").join($t).into(), FileIntent { source: PathBuf::from("source").join($s).into(), ..Default::default() })),*])
        };
    }

    #[test_case("" => HashMap::new(); "empty")]
    #[test_case(r#""a.txt" = "A.txt""# => mk_intent!(("a.txt", "A.txt")); "literal_file")]
    #[test_case(r#""subdir" = "dir""# => mk_intent!(("subdir/c.txt", "dir/c.txt"), ("subdir/d.txt", "dir/d.txt")); "subdir_to_dir")]
    #[test_case(r#""**/*.txt" = "files""# => mk_intent!(("a.txt", "files/a.txt"), ("b.txt", "files/b.txt"), ("subdir/c.txt", "files/subdir/c.txt"), ("subdir/d.txt", "files/subdir/d.txt")); "glob_deep")]
    #[test_case(r#""" = """# => mk_intent!(("a.txt", "a.txt"), ("b.txt", "b.txt"), ("subdir/c.txt", "subdir/c.txt"), ("subdir/d.txt", "subdir/d.txt")); "all_files")]
    #[test_case(r#""*.txt" = "root""# => mk_intent!(("a.txt", "root/a.txt"), ("b.txt", "root/b.txt")); "glob_root_only")]
    #[test_case(r#""./dir/../a.txt" = "./dir/../a.txt""# => mk_intent!(("a.txt", "a.txt")); "normalized_path")]
    #[test_case(r#""../../a.txt" = "../../a.txt""# => mk_intent!(("a.txt", "a.txt")); "parent_dir_ref")]
    #[test_case(r#""a.txt" = "A.txt"
"a.*" = """# => mk_intent!(("a.txt", "a.txt"), ("a.txt", "A.txt")); "multiple_portals_same_source")]
    fn test_resolve_portals(s: &str) -> HashMap<PathBuf, FileIntent> {
        let temp_dir = tempdir().unwrap();
        let source_dir = temp_dir.path().join("source");
        let target_dir = temp_dir.path().join("target");
        fs::create_dir(&source_dir).unwrap();
        fs::create_dir(&target_dir).unwrap();

        fs::write(source_dir.join("a.txt"), "").unwrap();
        fs::write(source_dir.join("b.txt"), "").unwrap();
        fs::create_dir(source_dir.join("subdir")).unwrap();
        fs::write(source_dir.join("subdir").join("c.txt"), "").unwrap();
        fs::write(source_dir.join("subdir").join("d.txt"), "").unwrap();

        fs::write(source_dir.join("dotrift.toml"), format!("[portal]\n{s}")).unwrap();

        let config = Config::read(source_dir.clone()).unwrap();
        let ignore_matcher = build_ignore(&config.ignore, &target_dir).unwrap();

        let intent =
            resolve_portals(&source_dir, &target_dir, &config.portal, &ignore_matcher).unwrap();

        let mut map = HashMap::new();
        for (p, f) in intent {
            map.insert(
                p.strip_prefix(temp_dir.path()).unwrap().to_path_buf(),
                FileIntent {
                    source: f
                        .source
                        .strip_prefix(temp_dir.path())
                        .unwrap()
                        .to_path_buf(),
                    ..f
                },
            );
        }

        map
    }

    #[test_case(r#""*.txt""# => mk_intent!(); "glob_no_match")]
    #[test_case(r#""subdir/*""# => mk_intent!(("a.txt", "a.txt"), ("b.txt", "b.txt")); "glob_subdir_only")]
    #[test_case(r#""**""# => mk_intent!(); "glob_all_empty")]
    #[test_case(r#""*.txt", "!dotrift.toml""# => mk_intent!(("dotrift.toml", "dotrift.toml")); "negate_ignore")]
    fn test_ignore(s: &str) -> HashMap<PathBuf, FileIntent> {
        let temp_dir = tempdir().unwrap();
        let source_dir = temp_dir.path().join("source");
        let target_dir = temp_dir.path().join("target");
        fs::create_dir(&source_dir).unwrap();
        fs::create_dir(&target_dir).unwrap();

        fs::write(source_dir.join("a.txt"), "").unwrap();
        fs::write(source_dir.join("b.txt"), "").unwrap();
        fs::create_dir(source_dir.join("subdir")).unwrap();
        fs::write(source_dir.join("subdir").join("c.txt"), "").unwrap();
        fs::write(source_dir.join("subdir").join("d.txt"), "").unwrap();

        fs::write(
            source_dir.join("dotrift.toml"),
            format!(
                r#"ignore = [{s}]
[portal]
"" = """#
            ),
        )
        .unwrap();

        let config = Config::read(source_dir.clone()).unwrap();
        let ignore_matcher = build_ignore(&config.ignore, &target_dir).unwrap();

        let intent =
            resolve_portals(&source_dir, &target_dir, &config.portal, &ignore_matcher).unwrap();

        let mut map = HashMap::new();
        for (p, f) in intent {
            map.insert(
                p.strip_prefix(temp_dir.path()).unwrap().to_path_buf(),
                FileIntent {
                    source: f
                        .source
                        .strip_prefix(temp_dir.path())
                        .unwrap()
                        .to_path_buf(),
                    ..f
                },
            );
        }

        map
    }

    #[test]
    fn test_resolve_portals_collision() {
        let temp_dir = tempdir().unwrap();
        let source_dir = temp_dir.path().join("source");
        let target_dir = temp_dir.path().join("target");
        fs::create_dir(&source_dir).unwrap();
        fs::create_dir(&target_dir).unwrap();

        fs::write(source_dir.join("a.txt"), "").unwrap();
        fs::write(source_dir.join("b.txt"), "").unwrap();

        fs::write(
            source_dir.join("dotrift.toml"),
            r#"[portal]
"a.txt" = "same.txt"
"b.txt" = "same.txt""#,
        )
        .unwrap();

        let config = Config::read(source_dir.clone()).unwrap();
        let ignore_matcher = build_ignore(&config.ignore, &target_dir).unwrap();

        let result = resolve_portals(
            &source_dir.normalize(),
            &target_dir.normalize(),
            &config.portal,
            &ignore_matcher,
        );

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("collision"));
    }
}
