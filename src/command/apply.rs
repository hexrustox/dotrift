use std::collections::HashMap;
use std::fs::{self, remove_file};
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, Result, eyre};
use dialoguer::Select;
use glob::glob;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use normalize_path::NormalizePath;
use walkdir::WalkDir;

use crate::cli::ApplyFlags;
use crate::command::tree::{Node, build_tree};
use crate::command::util::hash_file;
use crate::config::{Config, DeployType, FileMode, Rules};
use crate::db::{Db, DbEntry};
use crate::path::db_path;

#[derive(Default, Debug, PartialEq)]
pub struct PortalEntry {
    pub source: PathBuf,
    pub action_type: DeployType,
    pub mode: Option<FileMode>,
}

pub fn run(source_dir: PathBuf, target_override: Option<PathBuf>, flags: ApplyFlags) -> Result<()> {
    let source_normalized = source_dir.normalize();

    let config = Config::read(source_dir.clone())?;

    let target_dir = resolve_target(target_override, &config)?;
    let target_normalized = target_dir.normalize();

    validate_paths(&source_normalized, &target_normalized)?;

    let ignore_matcher = build_ignore(&config.ignore, &target_normalized)?;

    let mut portal_entries = resolve_portals(
        &source_normalized,
        &target_normalized,
        &config.portal,
        &ignore_matcher,
    )?;

    apply_rules(&mut portal_entries, &config.rule)?;

    let tree = build_tree(portal_entries)?;

    let db = Db::init(&db_path())?;
    execute_node(Path::new("/"), &tree, &db)?;

    Ok(())
}

fn execute_node(target: &Path, node: &Node, db: &Db) -> Result<()> {
    match node {
        Node::Dir(children) => {
            execute_dir(target, db)?;
            for (name, child) in children {
                execute_node(&target.join(name), child, db)?;
            }
        }
        Node::File(entry) => {
            write_file(target, entry, db)?;
        }
    }
    Ok(())
}

fn execute_dir(path: &Path, db: &Db) -> Result<()> {
    if path.exists() {
        if path.is_dir() {
            return Ok(());
        }
        let choice = prompt_collision(path, true)?;
        match choice {
            0 => return Ok(()),
            1 => {
                fs::remove_file(path)
                    .wrap_err_with(|| format!("Failed to remove `{}`.", path.display()))?;
                db.delete_entry(path)?;
            }
            2 => {
                return Err(eyre!("Aborted."));
            }
            _ => unreachable!(),
        }
    }
    fs::create_dir_all(path)
        .wrap_err_with(|| format!("Failed to create directory `{}`.", path.display()))?;
    Ok(())
}

fn write_file(target: &Path, entry: &PortalEntry, db: &Db) -> Result<()> {
    if target.exists() {
        if target.is_dir() {
            let choice = prompt_collision(target, false)?;
            match choice {
                0 => return Ok(()),
                1 => {
                    fs::remove_dir_all(target).wrap_err_with(|| {
                        format!("Failed to remove directory `{}`.", target.display())
                    })?;
                    db.delete_entry_with_prefix(target)?;
                }
                2 => {
                    return Err(eyre!("Aborted."));
                }
                _ => unreachable!(),
            }
        } else {
            let managed = is_managed(target, db);
            if !managed {
                let choice = prompt_collision(target, false)?;
                match choice {
                    0 => return Ok(()),
                    1 => {}
                    2 => {
                        return Err(eyre!("Aborted."));
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    deploy_file(target, entry, db)?;
    Ok(())
}

fn is_managed(target: &Path, db: &Db) -> bool {
    let db_entry = match db.get_entry(target).ok() {
        Some(Some(e)) => e,
        _ => return false,
    };

    match db_entry.action_type {
        DeployType::Symlink => match std::fs::read_link(target) {
            Ok(p) => p == db_entry.reference,
            Err(_) => false,
        },
        DeployType::Copy => match hash_file(target) {
            Ok(h) => Some(h) == db_entry.hash,
            Err(_) => false,
        },
    }
}

fn deploy_file(target: &Path, entry: &PortalEntry, db: &Db) -> Result<()> {
    match entry.action_type {
        DeployType::Symlink => {
            let _ = remove_file(target);
            unix_fs::symlink(&entry.source, target)
                .wrap_err_with(|| format!("Failed to create symlink `{}`.", target.display()))?;
        }
        DeployType::Copy => {
            fs::copy(&entry.source, target).wrap_err_with(|| {
                format!(
                    "Failed to copy `{}` to `{}`.",
                    entry.source.display(),
                    target.display()
                )
            })?;
            if let Some(mode) = entry.mode {
                let mode_val = mode.0 as u32;
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(target, fs::Permissions::from_mode(mode_val)).wrap_err_with(
                    || format!("Failed to set permissions on `{}`.", target.display()),
                )?;
            }
        }
    }

    db.insert_or_update(&DbEntry {
        target_path: target.to_path_buf(),
        action_type: entry.action_type,
        reference: entry.source.clone(),
        hash: if entry.action_type == DeployType::Copy {
            Some(hash_file(&entry.source)?)
        } else {
            None
        },
    })?;

    Ok(())
}

fn prompt_collision(path: &Path, is_dir: bool) -> Result<usize> {
    let type_str = if is_dir { "directory" } else { "file" };
    let selection = Select::new()
        .with_prompt(format!(
            "`{}` is an existing {}, skip/overwrite/quit?",
            path.display(),
            type_str
        ))
        .items(["skip", "overwrite", "quit"])
        .default(0)
        .interact()
        .wrap_err("Failed to get user input.")?;

    println!();
    Ok(selection)
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
) -> Result<HashMap<PathBuf, PortalEntry>> {
    let mut portal_entries: HashMap<PathBuf, PortalEntry> = HashMap::new();

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
                &mut portal_entries,
            )?;
        } else {
            resolve_literal_portal(
                source_dir,
                target_dir,
                &pattern_normalized,
                &target_rel_normalized,
                ignore_matcher,
                &mut portal_entries,
            )?;
        }
    }

    Ok(portal_entries)
}

fn resolve_glob_portal(
    source_dir: &Path,
    target_dir: &Path,
    pattern: &str,
    target_rel: &Path,
    ignore_matcher: &Gitignore,
    portal_entries: &mut HashMap<PathBuf, PortalEntry>,
) -> Result<()> {
    let prefix = stripping_prefix(pattern);
    let full_pattern = source_dir.join(pattern);
    let full_pattern_str = full_pattern.to_string_lossy();

    for entry in glob(&full_pattern_str).wrap_err("Invalid glob pattern.")? {
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

        if is_ignored(ignore_matcher, &target_path) {
            continue;
        }

        insert_portal_entry(portal_entries, target_path, source_path)?;
    }

    Ok(())
}

fn resolve_literal_portal(
    source_dir: &Path,
    target_dir: &Path,
    pattern: &Path,
    target_rel: &Path,
    ignore_matcher: &Gitignore,
    portal_entries: &mut HashMap<PathBuf, PortalEntry>,
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

        if is_ignored(ignore_matcher, &target_path) {
            return Ok(());
        }

        insert_portal_entry(portal_entries, target_path, source_path)?;
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

            if is_ignored(ignore_matcher, &target_path) {
                continue;
            }

            insert_portal_entry(portal_entries, target_path, file_source)?;
        }
    }

    Ok(())
}

fn insert_portal_entry(
    portal_entries: &mut HashMap<PathBuf, PortalEntry>,
    target_path: PathBuf,
    source_path: PathBuf,
) -> Result<()> {
    if let Some(existing) = portal_entries.insert(
        target_path.clone(),
        PortalEntry {
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

fn apply_rules(portal_entries: &mut HashMap<PathBuf, PortalEntry>, rules: &Rules) -> Result<()> {
    for (pattern, rule) in rules.iter().rev() {
        let pattern = glob::Pattern::new(pattern).wrap_err("Invalid glob pattern.")?;
        for (path, portal_entry) in portal_entries.iter_mut() {
            if !pattern.matches(&path.to_string_lossy()) {
                continue;
            }
            if let Some(rule_type) = &rule.r#type {
                portal_entry.action_type = *rule_type;
            }
            if let Some(rule_mode) = &rule.mode {
                portal_entry.mode = Some(*rule_mode);
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

    #[macro_export]
    macro_rules! portal_entries {
        ($(($s:literal, $t:literal)),*) => {
            HashMap::from_iter([$(($t.into(), PortalEntry { source: $s.into(), ..Default::default() })),*])
        };
        ($(($s:literal, $t:literal, $a:ident, $m:expr)),*) => {
            HashMap::from_iter([$(($t.into(), PortalEntry { source: $s.into(), action_type: DeployType::$a, mode: $m })),*])
        };
    }

    fn test_setup(
        portal: &str,
        ignore: Option<&str>,
        rule: Option<&str>,
    ) -> (tempfile::TempDir, PathBuf, PathBuf) {
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
        let config = format!(
            "ignore = [{}]\n[portal]\n{portal}\n[rule]\n{}",
            ignore.unwrap_or(""),
            rule.unwrap_or("")
        );
        fs::write(source_dir.join("dotrift.toml"), config).unwrap();
        (temp_dir, source_dir, target_dir)
    }

    fn flatten(
        map: HashMap<PathBuf, PortalEntry>,
        temp_dir: &Path,
    ) -> HashMap<PathBuf, PortalEntry> {
        map.into_iter()
            .map(|(p, f)| {
                (
                    p.strip_prefix(temp_dir.join("target"))
                        .unwrap()
                        .to_path_buf(),
                    PortalEntry {
                        source: f
                            .source
                            .strip_prefix(temp_dir.join("source"))
                            .unwrap()
                            .to_path_buf(),
                        ..f
                    },
                )
            })
            .collect()
    }

    #[test_case("" => HashMap::new(); "empty")]
    #[test_case(r#""a.txt" = "A.txt""# => portal_entries!(("a.txt", "A.txt")); "literal_file")]
    #[test_case(r#""subdir" = "dir""# => portal_entries!(("subdir/c.txt", "dir/c.txt"), ("subdir/d.txt", "dir/d.txt")); "subdir_to_dir")]
    #[test_case(r#""**/*.txt" = "files""# => portal_entries!(("a.txt", "files/a.txt"), ("b.txt", "files/b.txt"), ("subdir/c.txt", "files/subdir/c.txt"), ("subdir/d.txt", "files/subdir/d.txt")); "glob_deep")]
    #[test_case(r#""" = """# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt"), ("subdir/c.txt", "subdir/c.txt"), ("subdir/d.txt", "subdir/d.txt")); "all_files")]
    #[test_case(r#""*.txt" = "root""# => portal_entries!(("a.txt", "root/a.txt"), ("b.txt", "root/b.txt")); "glob_root_only")]
    #[test_case(r#""../../a.txt" = "../../a.txt""# => portal_entries!(("a.txt", "a.txt")); "parent_dir_ref")]
    #[test_case(r#""a.txt" = "A.txt"
"a.*" = """# => portal_entries!(("a.txt", "a.txt"), ("a.txt", "A.txt")); "multiple_portals_same_source")]
    fn test_resolve_portals(portal: &str) -> HashMap<PathBuf, PortalEntry> {
        let (temp_dir, source_dir, target_dir) = test_setup(portal, None, None);
        let config = Config::read(source_dir.clone()).unwrap();
        let ignore_matcher = build_ignore(&config.ignore, &target_dir).unwrap();
        let portal_entries =
            resolve_portals(&source_dir, &target_dir, &config.portal, &ignore_matcher).unwrap();
        flatten(portal_entries, temp_dir.path())
    }

    #[test_case(r#""*.txt""# => portal_entries!(); "glob_no_match")]
    #[test_case(r#""subdir/*""# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt")); "glob_subdir_only")]
    #[test_case(r#""**""# => portal_entries!(); "glob_all_empty")]
    #[test_case(r#""*.txt", "!dotrift.toml""# => portal_entries!(("dotrift.toml", "dotrift.toml")); "negate_ignore")]
    fn test_ignore(ignore: &str) -> HashMap<PathBuf, PortalEntry> {
        let (temp_dir, source_dir, target_dir) = test_setup("\"\" = \"\"", Some(ignore), None);
        let config = Config::read(source_dir.clone()).unwrap();
        let ignore_matcher = build_ignore(&config.ignore, &target_dir).unwrap();
        let portal_entries =
            resolve_portals(&source_dir, &target_dir, &config.portal, &ignore_matcher).unwrap();
        flatten(portal_entries, temp_dir.path())
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

    #[test_case(r#""*.txt" = { mode = "600" }"# => portal_entries!(("a.txt", "a.txt", Symlink, Some(FileMode(0o600)))); "rule_mode")]
    #[test_case(r#""*.txt" = { type = "copy" }"# => portal_entries!(("a.txt", "a.txt", Copy, None)); "rule_type")]
    #[test_case(r#""*.txt" = { mode = "600" }
"**/a.txt" = { type = "copy" }"# => portal_entries!(("a.txt", "a.txt", Copy, Some(FileMode(0o600)))); "rule_merge")]
    #[test_case(r#""*.txt" = { type = "symlink", mode = "600" }
"**/a.txt" = { type = "copy", mode = "700" }"# => portal_entries!(("a.txt", "a.txt", Copy, Some(FileMode(0o700)))); "rule_override")]
    fn test_apply_rules(rule: &str) -> HashMap<PathBuf, PortalEntry> {
        let (temp_dir, source_dir, target_dir) =
            test_setup(r#""a.txt" = "a.txt""#, None, Some(rule));
        let config = Config::read(source_dir.clone()).unwrap();
        let ignore_matcher = build_ignore(&config.ignore, &target_dir).unwrap();
        let mut portal_entries =
            resolve_portals(&source_dir, &target_dir, &config.portal, &ignore_matcher).unwrap();
        apply_rules(&mut portal_entries, &config.rule).unwrap();
        flatten(portal_entries, temp_dir.path())
    }

    #[test_case(|s, t| {
        unix_fs::symlink(s.join("file"), t.join("link")).unwrap();
    },
    |t| t.join("link"),
    |s, t| Some(DbEntry { target_path: t.join("link"), action_type: DeployType::Symlink, reference: s.join("file"), hash: None })
    => true; "symlink_matching_source")]
    #[test_case(|s, t| {
        fs::write(s.join("file"), "").unwrap();
        fs::write(t.join("file"), "").unwrap();
    },
    |t| t.join("file"),
    |s, t| Some(DbEntry { target_path: t.join("file"), action_type: DeployType::Copy, reference: PathBuf::new(), hash: Some(hash_file(&s.join("file")).unwrap()) })
    => true; "copy_matching_hash")]
    #[test_case(|s, t| {
        fs::write(s.join("file"), "").unwrap();
        unix_fs::symlink(s.join("file"), s.join("link")).unwrap();
        unix_fs::symlink(s.join("file"), t.join("link")).unwrap();
    },
    |t| t.join("link"),
    |s, t| Some(DbEntry { target_path: t.join("link"), action_type: DeployType::Copy, reference: PathBuf::new(), hash: Some(hash_file(&s.join("link")).unwrap()) })
    => true; "copy_symlink")]
    #[test_case(|s, t| {
        unix_fs::symlink(s.join("file1"), t.join("link")).unwrap();
    },
    |t| t.join("link"),
    |s, t| Some(DbEntry { target_path: t.join("link"), action_type: DeployType::Symlink, reference: s.join("file2"), hash: None })
    => false; "symlink_different_source")]
    #[test_case(|s, t| {
        fs::write(s.join("file"), "a").unwrap();
        fs::write(t.join("file"), "b").unwrap();
    },
    |t| t.join("file"),
    |s, t| Some(DbEntry { target_path: t.join("file"), action_type: DeployType::Copy, reference: PathBuf::new(), hash: Some(hash_file(&s.join("file")).unwrap()) })
    => false; "copy_different_hash")]
    #[test_case(|s, t| {
        unix_fs::symlink(s.join("file"), t.join("link")).unwrap();
    },
    |t| t.join("link"),
    |s, t| Some(DbEntry { target_path: t.join("link"), action_type: DeployType::Copy, reference: s.join("file"), hash: None })
    => false; "symlink_db_is_copy")]
    #[test_case(|s, t| {
        fs::write(s.join("file"), "").unwrap();
        fs::write(t.join("file"), "").unwrap();
    },
    |t| t.join("file"),
    |_, _| None
    => false; "no_db_entry")]
    #[test_case(|s, t| {
        fs::write(s.join("file"), "").unwrap();
        fs::write(t.join("file"), "").unwrap();
    },
    |t| t.join("file"),
    |s, t| Some(DbEntry { target_path: t.join("file"), action_type: DeployType::Symlink, reference: s.join("file"), hash: None })
    => false; "target_is_file_not_symlink")]
    fn test_is_managed(
        cb: impl FnOnce(&PathBuf, &PathBuf),
        target_path: impl FnOnce(&PathBuf) -> PathBuf,
        db_entry: impl FnOnce(&PathBuf, &PathBuf) -> Option<DbEntry>,
    ) -> bool {
        let temp_dir = tempdir().unwrap();
        let source = temp_dir.path().join("source");
        let target = temp_dir.path().join("target");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();

        cb(&source, &target);

        let db = Db::init(&temp_dir.path().join("db")).unwrap();
        if let Some(e) = db_entry(&source, &target) {
            db.insert_or_update(&e).unwrap();
        }

        is_managed(&target_path(&target), &db)
    }
}
