use std::{
    collections::HashMap,
    fs::{self, remove_file},
    os::unix::fs as unix_fs,
    path::{Path, PathBuf},
};

use color_eyre::eyre::{Context, Result, eyre};
use glob::{Pattern, glob_with};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use normalize_path::NormalizePath;
use walkdir::WalkDir;

use crate::{
    cli::ApplyFlags,
    command::{
        prompt::{CollisionOptions, prompt_collision},
        tree::{Node, build_tree},
        util::{
            GLOB_OPTION, clean_up, hash_file, is_glob, is_literal_dir, is_managed, print_portal,
            resolve_target, stripping_prefix, validate_paths,
        },
    },
    config::{Config, DeployType, FileMode, Rules},
    db::{Db, DbEntry},
    error::{GlobError, IoError},
    global_config::GlobalConfig,
    path::global_config_path,
};

#[derive(Default, Debug, PartialEq)]
pub struct PortalEntry {
    pub source: PathBuf,
    pub deploy_type: DeployType,
    pub mode: Option<FileMode>,
}

pub fn run(
    source_dir: PathBuf,
    target_override: Option<PathBuf>,
    config_override: Option<PathBuf>,
    db_path: &Path,
    flags: ApplyFlags,
) -> Result<()> {
    let config = Config::read(&source_dir)?;

    let target_dir = resolve_target(target_override, &config)?.normalize();

    validate_paths(&source_dir, &target_dir)?;

    let ignore_matcher =
        build_ignore(&config.ignore, &target_dir).wrap_err("Failed to build ignore matcher")?;

    let mut portal_entries =
        resolve_portals(&source_dir, &target_dir, &config.portal, &ignore_matcher)
            .wrap_err("Failed to resolve portals")?;

    apply_rules(&target_dir, &mut portal_entries, &config.rule)
        .wrap_err("Failed to apply rules")?;

    let db = Db::init(db_path)?;

    if flags.clean_up {
        clean_up(
            Some(&portal_entries),
            &db,
            flags.dry_run,
            flags.prune_empty_dirs,
        )?;
    }

    let tree = build_tree(portal_entries).wrap_err("Failed to build target file system tree")?;

    if flags.dry_run {
        print_tree(Path::new("/"), &tree)?;
        return Ok(());
    }

    let specific_config = config_override.is_some();
    let overwrite_identical =
        match GlobalConfig::read(&config_override.unwrap_or(global_config_path())) {
            Ok(config) => config.overwrite_identical,
            Err(e) if specific_config => return Err(e),
            _ => false,
        };
    traverse_tree(Path::new("/"), &tree, &db, overwrite_identical)?;

    Ok(())
}

fn build_ignore(patterns: &[String], target_dir: &Path) -> Result<Gitignore> {
    let mut builder = GitignoreBuilder::new(target_dir);
    builder
        .add_line(None, "dotrift.toml")
        .expect("Failed to add dotrift.toml ignore");
    for pattern in patterns {
        builder
            .add_line(None, pattern)
            .wrap_err("Invalid ignore pattern")?;
    }
    builder.build().wrap_err("Failed to build ignore matcher")
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

    for entry in glob_with(&full_pattern_str, GLOB_OPTION).glob_error()? {
        let source_path = entry.wrap_err("Failed to read glob match")?;
        if is_literal_dir(&source_path) {
            continue;
        }

        let source_rel = source_path.strip_prefix(source_dir).unwrap_or(&source_path);

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
            "Source path does not exist: `{}`",
            source_path.display()
        ));
    }

    if is_literal_dir(&source_path) {
        for entry in WalkDir::new(&source_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| !e.file_type().is_dir())
        {
            let file_source = entry.path().to_path_buf();

            let rel_to_pattern = file_source
                .strip_prefix(&source_path)
                .unwrap_or(&file_source);

            let target_path = target_dir.join(target_rel).join(rel_to_pattern);

            if is_ignored(ignore_matcher, &target_path) {
                continue;
            }

            insert_portal_entry(portal_entries, target_path, file_source)?;
        }
    } else {
        let target_path = target_dir.join(target_rel);

        if is_ignored(ignore_matcher, &target_path) {
            return Ok(());
        }

        insert_portal_entry(portal_entries, target_path, source_path)?;
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
            deploy_type: DeployType::default(),
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
    matcher.matched_path_or_any_parents(path, false).is_ignore()
}

fn apply_rules(
    target_dir: &Path,
    portal_entries: &mut HashMap<PathBuf, PortalEntry>,
    rules: &Rules,
) -> Result<()> {
    for (pattern, rule) in rules.iter() {
        let pattern = Pattern::new(pattern).glob_error()?;
        for (path, portal_entry) in portal_entries.iter_mut() {
            if !pattern
                .matches_path_with(path.strip_prefix(target_dir).unwrap_or(path), GLOB_OPTION)
            {
                continue;
            }
            if let Some(rule_type) = &rule.r#type {
                portal_entry.deploy_type = *rule_type;
            }
            if let Some(rule_mode) = &rule.mode {
                portal_entry.mode = Some(*rule_mode);
            }
        }
    }

    Ok(())
}

fn print_tree(path: &Path, node: &Node) -> Result<()> {
    match node {
        Node::Dir(children) => {
            if path != Path::new("/") {
                println!("[CREATE] {}", path.display());
            }
            for (name, child) in children {
                print_tree(&path.join(name), child)?;
            }
        }
        Node::File(entry) => {
            println!(
                "[CREATE] {}",
                print_portal(path, &entry.source, entry.deploy_type)
            );
        }
    }
    Ok(())
}

fn traverse_tree(target: &Path, node: &Node, db: &Db, overwrite_identical: bool) -> Result<()> {
    match node {
        Node::Dir(children) => {
            if create_dir(target, db)? {
                return Ok(());
            }
            for (name, child) in children {
                traverse_tree(&target.join(name), child, db, overwrite_identical)?;
            }
        }
        Node::File(entry) => {
            write_file(target, entry, db, overwrite_identical)?;
        }
    }
    Ok(())
}

fn create_dir(path: &Path, db: &Db) -> Result<bool> {
    if path.exists() {
        if is_literal_dir(path) {
            return Ok(false);
        }
        let choice = prompt_collision(path, true)?;
        match choice {
            CollisionOptions::Skip => return Ok(true),
            CollisionOptions::Overwrite => {
                fs::remove_file(path).remove_file_error(path)?;
                db.delete_entry(path)?;
            }
            CollisionOptions::Quit => {
                return Err(eyre!("Aborted"));
            }
        }
    }
    fs::create_dir_all(path).create_dir_error(path)?;
    Ok(false)
}

fn write_file(
    target: &Path,
    entry: &PortalEntry,
    db: &Db,
    overwrite_identical: bool,
) -> Result<()> {
    if target.exists() {
        if is_literal_dir(target) {
            let choice = prompt_collision(target, false)?;
            match choice {
                CollisionOptions::Skip => return Ok(()),
                CollisionOptions::Overwrite => {
                    fs::remove_dir_all(target).remove_dir_error(target)?;
                    db.delete_entry_with_prefix(target)?;
                }
                CollisionOptions::Quit => {
                    return Err(eyre!("Aborted"));
                }
            }
        } else {
            let mut target_hash = None;
            let identical = match entry.deploy_type {
                DeployType::Symlink => {
                    target.is_symlink() && fs::read_link(target).is_ok_and(|l| l == entry.source)
                }
                DeployType::Copy => {
                    entry
                        .source
                        .metadata()
                        .is_ok_and(|m1| target.metadata().is_ok_and(|m2| m1.len() == m2.len()))
                        && hash_file(target).is_ok_and(|h1| {
                            target_hash = Some(h1);
                            hash_file(&entry.source).is_ok_and(|h2| h1 == h2)
                        })
                }
            };
            if identical {
                if overwrite_identical {
                    update_db(target, entry, db)?;
                }
                return Ok(());
            }

            #[cfg(test)]
            {
                tests::CHECK_MANAGED.set(true);
            }
            let managed = is_managed(target, db, target_hash);
            if !managed {
                let choice = prompt_collision(target, false)?;
                match choice {
                    CollisionOptions::Skip => return Ok(()),
                    CollisionOptions::Overwrite => {}
                    CollisionOptions::Quit => {
                        return Err(eyre!("Aborted"));
                    }
                }
            }
        }
    }

    match entry.deploy_type {
        DeployType::Symlink => {
            let _ = remove_file(target);
            unix_fs::symlink(&entry.source, target)
                .wrap_err_with(|| format!("Failed to create symlink `{}`", target.display()))?;
        }
        DeployType::Copy => {
            fs::copy(&entry.source, target).copy_file_error(&entry.source, target)?;
            if let Some(mode) = entry.mode {
                let mode_val = mode.0 as u32;
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(target, fs::Permissions::from_mode(mode_val)).wrap_err_with(
                    || format!("Failed to set permissions on `{}`", target.display()),
                )?;
            }
        }
    }
    update_db(target, entry, db)?;
    Ok(())
}

fn update_db(target: &Path, entry: &PortalEntry, db: &Db) -> Result<()> {
    db.insert_or_update(&DbEntry {
        deploy_type: entry.deploy_type,
        source_path: entry.source.clone(),
        hash: if entry.deploy_type == DeployType::Copy {
            Some(hash_file(target)?)
        } else {
            None
        },
        target_path: target.to_path_buf(),
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::command::{prompt::tests::PROMPT_SELECTION, util::tests::setup_test};

    use super::*;
    use std::{cell::RefCell, fs};
    use test_case::test_case;

    #[macro_export]
    macro_rules! portal_entries {
        ($(($s:literal, $t:literal)),*) => {
            HashMap::from_iter([$(($t.into(), PortalEntry { source: $s.into(), ..Default::default() })),*])
        };
        ($(($s:literal, $t:literal, $a:ident, $m:expr)),*) => {
            HashMap::from_iter([$(($t.into(), PortalEntry { source: $s.into(), deploy_type: DeployType::$a, mode: $m })),*])
        };
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

    #[test_case("" => HashMap::new(); "literal_empty")]
    #[test_case(r#""a.txt" = "A.txt""# => portal_entries!(("a.txt", "A.txt")); "literal_file")]
    #[test_case(r#""subdir" = "dir""# => portal_entries!(("subdir/c.txt", "dir/c.txt"), ("subdir/d.txt", "dir/d.txt")); "literal_dir")]
    #[test_case(r#""" = """# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt"), ("subdir/c.txt", "subdir/c.txt"), ("subdir/d.txt", "subdir/d.txt")); "literal_root")]
    #[test_case(r#""./a.txt" = "./b.txt""# => portal_entries!(("a.txt", "b.txt")); "literal_dot_normalized")]
    #[test_case(r#""subdir/dir/../c.txt" = "dist/../root/c.txt""# => portal_entries!(("subdir/c.txt", "root/c.txt")); "literal_path_traversal")]
    #[test_case(r#""../../a.txt" = "../../a.txt""# => portal_entries!(("a.txt", "a.txt")); "literal_path_escape_clamped")]
    #[test_case(r#""*.rs" = """# => portal_entries!(); "glob_no_match")]
    #[test_case(r#""*.txt" = "root""# => portal_entries!(("a.txt", "root/a.txt"), ("b.txt", "root/b.txt")); "glob_shallow_pattern")]
    #[test_case(r#""**/*" = ".""# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt"), ("subdir/c.txt", "subdir/c.txt"), ("subdir/d.txt", "subdir/d.txt")); "glob_recursive_all")]
    #[test_case(r#""**/*.txt" = "files""# => portal_entries!(("a.txt", "files/a.txt"), ("b.txt", "files/b.txt"), ("subdir/c.txt", "files/subdir/c.txt"), ("subdir/d.txt", "files/subdir/d.txt")); "glob_recursive_pattern")]
    #[test_case(r#""**/c.txt" = "out""# => portal_entries!(("subdir/c.txt", "out/subdir/c.txt")); "glob_recursive_prefix")]
    #[test_case(r#""subdir/**/*.txt" = "out""# => portal_entries!(("subdir/c.txt", "out/c.txt"), ("subdir/d.txt", "out/d.txt")); "glob_recursive_middle")]
    #[test_case(r#""a.txt" = "A.txt"
"a.*" = """# => portal_entries!(("a.txt", "a.txt"), ("a.txt", "A.txt")); "multiple_same_source")]
    #[test_case(r#""a.txt" = "same"
"b.txt" = "same""# => panics "collision"; "multiple_same_target")]
    #[test_case(r#""foo" = "bar""# => panics "does not exist"; "non_existing")]
    fn test_resolve_portals(portal: &str) -> HashMap<PathBuf, PortalEntry> {
        let (temp_dir, source_dir, target_dir) = setup_test(portal, "", "", true);
        let config = Config::read(&source_dir).unwrap();
        let ignore_matcher = build_ignore(&config.ignore, &target_dir).unwrap();
        let portal_entries =
            resolve_portals(&source_dir, &target_dir, &config.portal, &ignore_matcher).unwrap();
        flatten(portal_entries, temp_dir.path())
    }

    #[test_case("" => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt"), ("subdir/c.txt", "subdir/c.txt"), ("subdir/d.txt", "subdir/d.txt")); "empty")]
    #[test_case(r#""*.txt""# => portal_entries!(); "glob_all_files")]
    #[test_case(r#""**""# => portal_entries!(); "glob_everything")]
    #[test_case(r#""/*.txt""# => portal_entries!(("subdir/c.txt", "subdir/c.txt"), ("subdir/d.txt", "subdir/d.txt")); "glob_anchored")]
    #[test_case(r#""subdir/*""# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt")); "glob_dir_contents")]
    #[test_case(r#""**/c.txt""# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt"), ("subdir/d.txt", "subdir/d.txt")); "glob_file_anywhere")]
    #[test_case(r#""subdir/**""# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt")); "glob_double_star_dir")]
    #[test_case(r#""subdir/""# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt")); "dir_trailing_slash")]
    #[test_case(r#""*.txt", "!dotrift.toml""# => portal_entries!(("dotrift.toml", "dotrift.toml")); "negate_selective")]
    #[test_case(r#""!a.txt""# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt"), ("subdir/c.txt", "subdir/c.txt"), ("subdir/d.txt", "subdir/d.txt")); "negate_only")]
    #[test_case(r#""*.txt", "!a.txt", "!b.txt""# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt")); "negate_multiple")]
    #[test_case(r#""!nonexistent.txt""# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt"), ("subdir/c.txt", "subdir/c.txt"), ("subdir/d.txt", "subdir/d.txt")); "negate_nonexistent")]
    #[test_case(r#""a.txt", "b.txt""# => portal_entries!(("subdir/c.txt", "subdir/c.txt"), ("subdir/d.txt", "subdir/d.txt")); "multiple_literal")]
    fn test_ignore(ignore: &str) -> HashMap<PathBuf, PortalEntry> {
        let (temp_dir, source_dir, target_dir) = setup_test(r#""" = """#, ignore, "", true);
        let config = Config::read(&source_dir).unwrap();
        let ignore_matcher = build_ignore(&config.ignore, &target_dir).unwrap();
        let portal_entries =
            resolve_portals(&source_dir, &target_dir, &config.portal, &ignore_matcher).unwrap();
        flatten(portal_entries, temp_dir.path())
    }

    #[test_case("" => portal_entries!(("a.txt", "a.txt", Symlink, None)); "empty")]
    #[test_case(r#""*.txt" = { mode = "600" }"# => portal_entries!(("a.txt", "a.txt", Symlink, Some(FileMode(0o600)))); "rule_mode")]
    #[test_case(r#""*.txt" = { type = "copy" }"# => portal_entries!(("a.txt", "a.txt", Copy, None)); "rule_type")]
    #[test_case(r#""*.txt" = { mode = "600" }
"a.txt" = { type = "copy" }"# => portal_entries!(("a.txt", "a.txt", Copy, Some(FileMode(0o600)))); "rule_merge")]
    #[test_case(r#""*.txt" = { type = "symlink", mode = "600" }
"a.txt" = { type = "copy", mode = "700" }"# => portal_entries!(("a.txt", "a.txt", Copy, Some(FileMode(0o700)))); "rule_override")]
    fn test_apply_rules(rule: &str) -> HashMap<PathBuf, PortalEntry> {
        let (temp_dir, source_dir, target_dir) = setup_test(r#""a.txt" = "a.txt""#, "", rule, true);
        let config = Config::read(&source_dir).unwrap();
        let ignore_matcher = build_ignore(&config.ignore, &target_dir).unwrap();
        let mut portal_entries =
            resolve_portals(&source_dir, &target_dir, &config.portal, &ignore_matcher).unwrap();
        apply_rules(&target_dir, &mut portal_entries, &config.rule).unwrap();
        flatten(portal_entries, temp_dir.path())
    }

    thread_local! {
        pub static CHECK_MANAGED: RefCell<bool> = const { RefCell::new(false) };
    }

    #[test_case(|s, _| {
        fs::write(s.join("file"), "").unwrap();
    }, |s, t| {
        assert!(t.join("file").exists());
        assert!(t.join("file").is_symlink());
        assert_eq!(fs::read_link(t.join("file")).unwrap(), s.join("file"));
    }; "simple")]
    #[test_case(|s, _| {
        fs::create_dir_all(s.join("dir")).unwrap();
        fs::write(s.join("dir/file"), "").unwrap();
    }, |_, t| {
        assert!(t.join("dir").exists());
        assert!(t.join("dir/file").exists());
    }; "symlink_nested_dirs")]
    #[test_case(|s, t| {
        fs::create_dir_all(s.join("dir")).unwrap();
        fs::write(s.join("dir/file"), "").unwrap();
        fs::write(t.join("dir"), "").unwrap();
    }, |_, t| {
        assert!(t.join("dir").exists());
        assert!(t.join("dir").is_file());
    }; "symlink_dir_blocked_by_file_skip")]
    #[test_case(|s, t| {
        PROMPT_SELECTION.set(CollisionOptions::Overwrite);
        fs::create_dir_all(s.join("dir")).unwrap();
        fs::write(s.join("dir/file"), "").unwrap();
        fs::write(t.join("dir"), "").unwrap();
    }, |_, t| {
        assert!(t.join("dir").exists());
        assert!(t.join("dir/file").exists());
    }; "symlink_dir_blocked_by_file_overwrite")]
    #[test_case(|s, t| {
        fs::write(s.join("dir"), "").unwrap();
        fs::create_dir_all(t.join("dir")).unwrap();
        fs::write(t.join("dir/file"), "").unwrap();
    }, |_, t| {
        assert!(t.join("dir").exists());
        assert!(t.join("dir/file").exists());
    }; "symlink_blocked_by_dir_skip")]
    #[test_case(|s, t| {
        PROMPT_SELECTION.set(CollisionOptions::Overwrite);
        fs::write(s.join("dir"), "").unwrap();
        fs::create_dir_all(t.join("dir")).unwrap();
        fs::write(t.join("dir/file"), "").unwrap();
    }, |_, t| {
        assert!(t.join("dir").exists());
        assert!(t.join("dir").is_file());
    }; "symlink_blocked_by_dir_overwrite")]
    #[test_case(|s, t| {
        fs::write(s.join("file"), "").unwrap();
        fs::write(t.join("file"), "").unwrap();
    }, |_, t| {
        assert!(!t.join("file").is_symlink());
    }; "symlink_blocked_by_existing_skip")]
    #[test_case(|s, t| {
        PROMPT_SELECTION.set(CollisionOptions::Overwrite);
        fs::write(s.join("file"), "").unwrap();
        fs::write(t.join("file"), "").unwrap();
    }, |_, t| {
        assert!(t.join("file").is_symlink());
    }; "symlink_blocked_by_existing_overwrite")]
    #[test_case(|s, _| {
        unix_fs::symlink(Path::new("/a"), s.join("file")).unwrap();
    }, |s, t| {
        assert_eq!(fs::read_link(t.join("file")).unwrap(), s.join("file"));
    }; "symlink_broken_preserved")]
    #[test_case(|s, t| {
        unix_fs::symlink(Path::new("/a"), s.join("file")).unwrap();
        unix_fs::symlink(s.join("file"), t.join("file")).unwrap();
    }, |_, _| {
        assert!(!CHECK_MANAGED.with_borrow(|b| *b));
    }; "identical_symlink")]
    fn test_apply_symlink(setup: impl FnOnce(&Path, &Path), assert: impl FnOnce(&Path, &Path)) {
        let (temp_dir, source_dir, target_dir) = setup_test(r#""" = """#, "", "", false);
        setup(&source_dir, &target_dir);
        run(
            source_dir.clone(),
            Some(target_dir.clone()),
            None,
            &temp_dir.path().join("db"),
            ApplyFlags {
                dry_run: false,
                clean_up: false,
                prune_empty_dirs: false,
            },
        )
        .unwrap();
        assert(&source_dir, &target_dir);
    }

    #[test_case(|s, _| {
        fs::write(s.join("file"), "").unwrap();
    }, |_, t| {
        use std::os::unix::fs::PermissionsExt;

        assert!(t.join("file").exists());
        assert!(!t.join("file").is_symlink());
        assert_eq!(fs::File::open(t.join("file")).unwrap().metadata().unwrap().permissions().mode(), 0o100123);
    }; "copy_with_mode")]
    #[test_case(|s, t| {
        fs::write(s.join("file"), "a").unwrap();
        fs::write(t.join("file"), "b").unwrap();
    }, |_, t| {
        assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "b");
    }; "copy_blocked_by_existing_skip")]
    #[test_case(|s, t| {
        PROMPT_SELECTION.set(CollisionOptions::Overwrite);
        fs::write(s.join("file"), "a").unwrap();
        fs::write(t.join("file"), "b").unwrap();
    }, |_, t| {
        assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "a");
    }; "copy_blocked_by_existing_overwrite")]
    #[test_case(|s, t| {
        fs::write(t.join("origin"), "").unwrap();
        unix_fs::symlink(t.join("origin"), s.join("file")).unwrap();
        assert!(s.join("file").is_symlink());
    }, |_, t| {
        assert!(t.join("file").exists());
        assert!(!t.join("file").is_symlink());
    }; "copy_overwrites_existing_symlink")]
    #[test_case(|s, t| {
        fs::write(s.join("file"), "a").unwrap();
        fs::write(t.join("file"), "a").unwrap();
    }, |_, _| {
        assert!(!CHECK_MANAGED.with_borrow(|b| *b));
    }; "identical_file")]
    fn test_apply_copy(setup: impl FnOnce(&Path, &Path), assert: impl FnOnce(&Path, &Path)) {
        let (temp_dir, source_dir, target_dir) = setup_test(
            r#""" = """#,
            "",
            r#""**/*" = { type = "copy", mode = "123" }"#,
            false,
        );
        setup(&source_dir, &target_dir);
        run(
            source_dir.clone(),
            Some(target_dir.clone()),
            None,
            &temp_dir.path().join("db"),
            ApplyFlags {
                dry_run: false,
                clean_up: false,
                prune_empty_dirs: false,
            },
        )
        .unwrap();
        assert(&source_dir, &target_dir);
    }

    #[test]
    fn test_dry_run_no_write() {
        let (temp_dir, source_dir, target_dir) = setup_test(r#""" = """#, "", "", false);
        fs::write(source_dir.join("file"), "").unwrap();

        run(
            source_dir.clone(),
            Some(target_dir.clone()),
            None,
            &temp_dir.path().join("db"),
            ApplyFlags {
                dry_run: true,
                clean_up: false,
                prune_empty_dirs: false,
            },
        )
        .unwrap();

        assert!(!target_dir.join("file").exists(),);
    }
}
