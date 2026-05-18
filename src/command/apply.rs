use std::{
    collections::HashMap,
    fs::{self, remove_file},
    os::unix::fs::{self as unix_fs, PermissionsExt},
    path::{Path, PathBuf},
};

use color_eyre::{
    Section,
    eyre::{Context, Result, eyre},
};
use glob::{Pattern, glob_with};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use normalize_path::NormalizePath;

use crate::{
    cli::{ApplyFlags, GlobalFlags},
    command::{
        prompt::{CollisionOptions, prompt_collision},
        tree::{Node, build_tree},
        util::{
            GLOB_OPTION, PathLiteral, SafeStripPrefix, clean_up, clone_file, hash_file, is_glob,
            is_managed, resolve_target, strip_prefix_filter_glob, walk_files,
        },
    },
    config::{Config, DeployType, FileMode, Rules},
    db::{Db, DbEntry},
    global_config::GlobalConfig,
    output,
};

#[derive(Default, Debug, PartialEq)]
pub struct PortalEntry {
    pub source: PathBuf,
    pub deploy_type: DeployType,
    pub mode: Option<FileMode>,
}

pub fn run(global_flags: GlobalFlags, db_path: &Path, flags: ApplyFlags) -> Result<()> {
    let source_dir = global_flags.source()?;
    let target_override = global_flags.target()?;
    let config_override = global_flags.config()?;

    let config = Config::read(&source_dir)?;

    let target_dir = resolve_target(&source_dir, target_override, &config)?;

    let ignore_matcher = build_ignore(&config.ignore, &target_dir)?;

    let mut portal_entries =
        resolve_portals(&source_dir, &target_dir, &config.portal, &ignore_matcher)?;

    apply_rules(&target_dir, &mut portal_entries, &config.rule)?;

    let db = Db::init(db_path)?;

    let remove_count = if flags.clean_up {
        clean_up(
            Some(&portal_entries),
            &db,
            flags.dry_run,
            flags.prune_empty_dirs,
        )?
    } else {
        0
    };

    let tree = build_tree(portal_entries).suggestion("Check portal entries for conflicting target paths. A file and a directory cannot share the same target path.")?;

    if flags.dry_run {
        let create_count = print_tree(Path::new("/"), &tree)?;
        let mut parts = Vec::with_capacity(2);
        if create_count > 0 {
            parts.push(if create_count == 1 {
                "1 create".to_string()
            } else {
                format!("{} creates", create_count)
            });
        }
        if remove_count > 0 {
            parts.push(if remove_count == 1 {
                "1 removal".to_string()
            } else {
                format!("{} removals", remove_count)
            });
        }
        if !parts.is_empty() {
            output::print_summary(parts.join(", "));
        }
        return Ok(());
    }

    let overwrite_identical = GlobalConfig::read(config_override)?.overwrite_identical;
    traverse_tree(Path::new("/"), &tree, &db, overwrite_identical)?;

    Ok(())
}

pub fn build_ignore(patterns: &[String], target_dir: &Path) -> Result<Gitignore> {
    let mut builder = GitignoreBuilder::new(target_dir);

    #[allow(unused_variables)]
    let result = builder.add_line(None, "dotrift.toml");
    #[cfg(test)]
    result.unwrap_or_else(|_| panic!("Failed to add dotrift.toml ignore"));

    for pattern in patterns {
        builder
            .add_line(None, pattern)
            .wrap_err_with(|| format!("Invalid ignore pattern: `{pattern}`"))
            .note("Use gitignore-style patterns. See gitignore documentation for syntax.")?;
    }
    builder
        .build()
        .wrap_err("Failed to compile ignore patterns")
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
        let target_rel_normalized = target_rel.normalize();

        if is_glob(&pattern_normalized.to_string_lossy()) {
            resolve_glob_portal(
                source_dir,
                target_dir,
                &pattern_normalized,
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
    pattern: &Path,
    target_rel: &Path,
    ignore_matcher: &Gitignore,
    portal_entries: &mut HashMap<PathBuf, PortalEntry>,
) -> Result<()> {
    let prefix = strip_prefix_filter_glob(&pattern.to_string_lossy());
    let full_pattern = source_dir.join(pattern);
    let full_pattern_str = full_pattern.to_string_lossy();

    for source_path in
        crate::glob_err!(glob_with(&full_pattern_str, GLOB_OPTION), &full_pattern_str)?.flatten()
    {
        if source_path.path_is_dir() {
            continue;
        }

        let source_rel = source_path.safe_strip_prefix(source_dir);

        let stripped = if prefix.is_empty() {
            source_rel.to_path_buf()
        } else {
            source_rel.safe_strip_prefix(&prefix).to_path_buf()
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

    if !source_path.path_exists() {
        return Err(eyre!(
            "Source path does not exist: `{}`",
            source_path.display()
        ));
    }

    if source_path.path_is_dir() {
        for entry in walk_files(&source_path) {
            let file_source = entry.path().to_path_buf();

            let rel_to_pattern = file_source.safe_strip_prefix(&source_path);

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

pub fn is_ignored(matcher: &Gitignore, path: &Path) -> bool {
    matcher.matched_path_or_any_parents(path, false).is_ignore()
}

fn apply_rules(
    target_dir: &Path,
    portal_entries: &mut HashMap<PathBuf, PortalEntry>,
    rules: &Rules,
) -> Result<()> {
    let mut compiled = Vec::with_capacity(rules.len());
    for (p, r) in rules {
        compiled.push((crate::glob_err!(Pattern::new(p), p)?, r));
    }

    for (path, portal_entry) in portal_entries.iter_mut() {
        let rel = path.safe_strip_prefix(target_dir); // computed E times (was R×E)
        for (pattern, rule) in &compiled {
            if !pattern.matches_path_with(rel, GLOB_OPTION) {
                continue;
            }
            if let Some(t) = &rule.deploy_type {
                portal_entry.deploy_type = *t;
            }
            if let Some(m) = &rule.mode {
                portal_entry.mode = Some(*m);
            }
        }
    }
    Ok(())
}

fn print_tree(path: &Path, node: &Node) -> Result<usize> {
    let mut count = 0;
    match node {
        Node::Dir(children) => {
            if path != Path::new("/") {
                output::print_created_dir(path);
            }
            for (name, child) in children {
                count += print_tree(&path.join(name), child)?;
            }
        }
        Node::File(entry) => {
            count += 1;
            output::print_created_file(path, &entry.source, entry.deploy_type);
        }
        Node::Marked(_) => {
            unreachable!()
        }
    }

    Ok(count)
}

fn traverse_tree(target: &Path, node: &Node, db: &Db, overwrite_identical: bool) -> Result<()> {
    match node {
        Node::Dir(children) => {
            if deploy_dir(target, db)? {
                return Ok(());
            }
            for (name, child) in children {
                traverse_tree(&target.join(name), child, db, overwrite_identical)?;
            }
        }
        Node::File(entry) => {
            deploy_file(target, entry, db, overwrite_identical)?;
        }
        Node::Marked(_) => {
            unreachable!()
        }
    }
    Ok(())
}

fn abort_deploy(at: &Path) -> color_eyre::Report {
    eyre!("Aborted at `{}`", at.display())
        .note("Not all files were deployed. Files deployed before this point remain in place.")
}

fn deploy_dir(path: &Path, db: &Db) -> Result<bool> {
    if path.path_exists() {
        if path.path_is_dir() {
            return Ok(false);
        }
        let choice = prompt_collision(path, true)?;
        match choice {
            CollisionOptions::Skip => return Ok(true),
            CollisionOptions::Overwrite => {
                crate::remove_file_err!(fs::remove_file(path), path)?;
                db.delete_entry(path)?;
            }
            CollisionOptions::Quit => {
                return Err(abort_deploy(path));
            }
        }
    }
    crate::create_dir_err!(fs::create_dir_all(path), path)?;
    Ok(false)
}

fn is_identical(
    target: &Path,
    source: &Path,
    deploy_type: DeployType,
    source_hash: &mut Option<u64>,
    target_hash: &mut Option<u64>,
) -> bool {
    match deploy_type {
        DeployType::Symlink => {
            target.path_is_symlink() && fs::read_link(target).is_ok_and(|l| l == source)
        }
        DeployType::Copy if source.path_is_symlink() => {
            target.path_is_symlink()
                && fs::read_link(source)
                    .is_ok_and(|src_dest| fs::read_link(target).is_ok_and(|l| l == src_dest))
        }
        DeployType::Copy => {
            source.path_is_file()
                && target.path_is_file()
                && hash_file(source).is_ok_and(|h1| {
                    *source_hash = Some(h1);
                    hash_file(target).is_ok_and(|h2| {
                        *target_hash = Some(h2);
                        h1 == h2
                    })
                })
        }
    }
}

fn deploy_file(
    target: &Path,
    entry: &PortalEntry,
    db: &Db,
    overwrite_identical: bool,
) -> Result<()> {
    let mut source_hash = None;
    let mut target_hash = None;

    if target.path_exists() {
        if target.path_is_dir() {
            let choice = prompt_collision(target, false)?;
            match choice {
                CollisionOptions::Skip => return Ok(()),
                CollisionOptions::Overwrite => {
                    crate::remove_dir_err!(fs::remove_dir_all(target), target)?;
                    db.delete_entry_with_prefix(target)?;
                }
                CollisionOptions::Quit => {
                    return Err(abort_deploy(target));
                }
            }
        } else {
            let identical = is_identical(
                target,
                &entry.source,
                entry.deploy_type,
                &mut source_hash,
                &mut target_hash,
            );
            if identical {
                if overwrite_identical {
                    update_db(target, entry, db, target_hash)?;
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
                        return Err(abort_deploy(target));
                    }
                }
            }
        }
    }

    match entry.deploy_type {
        DeployType::Symlink => {
            let _ = remove_file(target);
            crate::symlink_err!(
                unix_fs::symlink(&entry.source, target),
                target,
                &entry.source
            )?
        }
        DeployType::Copy => {
            clone_file(&entry.source, target)?;
            if let Some(mode) = entry.mode
                && target.path_is_file()
            {
                fs::set_permissions(target, fs::Permissions::from_mode(mode.0 as u32))
                    .wrap_err_with(|| {
                        format!("Failed to set permissions on `{}`", target.display())
                    })?;
            }
        }
    }

    update_db(target, entry, db, source_hash)?;
    Ok(())
}

fn update_db(target: &Path, entry: &PortalEntry, db: &Db, source_hash: Option<u64>) -> Result<()> {
    db.insert_or_update(&DbEntry {
        deploy_type: entry.deploy_type,
        source_path: entry.source.clone(),
        hash: if target.path_is_file() {
            Some(
                source_hash
                    .map(Ok)
                    .unwrap_or_else(|| hash_file(&entry.source))?,
            )
        } else {
            None
        },
        symlink_target: if entry.deploy_type == DeployType::Copy && entry.source.path_is_symlink() {
            Some(crate::read_link_err!(
                fs::read_link(&entry.source),
                &entry.source
            )?)
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
    use tempfile::tempdir;
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
    #[test_case(r#""**/*" = "."
"a.txt" = "a.txt""# => panics "collision"; "literal_glob_same_target")]
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

    #[test_case("" => portal_entries!(
        ("a.txt",         "a.txt",         Symlink, None),
        ("b.txt",         "b.txt",         Symlink, None),
        ("subdir/c.txt",  "subdir/c.txt",  Symlink, None),
        ("subdir/d.txt",  "subdir/d.txt",  Symlink, None)
    ); "empty")]
    #[test_case(r#""*.txt" = { type = "copy" }"# => portal_entries!(
        ("a.txt",         "a.txt",         Copy,    None),
        ("b.txt",         "b.txt",         Copy,    None),
        ("subdir/c.txt",  "subdir/c.txt",  Symlink, None),
        ("subdir/d.txt",  "subdir/d.txt",  Symlink, None)
    ); "selective_type")]
    #[test_case(r#""*.txt" = { mode = "600" }"# => portal_entries!(
        ("a.txt",         "a.txt",         Symlink, Some(FileMode(0o600))),
        ("b.txt",         "b.txt",         Symlink, Some(FileMode(0o600))),
        ("subdir/c.txt",  "subdir/c.txt",  Symlink, None),
        ("subdir/d.txt",  "subdir/d.txt",  Symlink, None)
    ); "rule_mode")]
    #[test_case(r#""*.txt" = { mode = "600" }
    "a.txt" = { type = "copy" }"# => portal_entries!(
        ("a.txt",         "a.txt",         Copy,    Some(FileMode(0o600))),
        ("b.txt",         "b.txt",         Symlink, Some(FileMode(0o600))),
        ("subdir/c.txt",  "subdir/c.txt",  Symlink, None),
        ("subdir/d.txt",  "subdir/d.txt",  Symlink, None)
    ); "rule_merge")]
    #[test_case(r#""*.txt" = { type = "symlink", mode = "600" }
    "a.txt" = { type = "copy", mode = "700" }"# => portal_entries!(
        ("a.txt",         "a.txt",         Copy,    Some(FileMode(0o700))),
        ("b.txt",         "b.txt",         Symlink, Some(FileMode(0o600))),
        ("subdir/c.txt",  "subdir/c.txt",  Symlink, None),
        ("subdir/d.txt",  "subdir/d.txt",  Symlink, None)
    ); "rule_override")]
    #[test_case(r#""subdir/*.txt" = { mode = "600" }"# => portal_entries!(
        ("a.txt",         "a.txt",         Symlink, None),
        ("b.txt",         "b.txt",         Symlink, None),
        ("subdir/c.txt",  "subdir/c.txt",  Symlink, Some(FileMode(0o600))),
        ("subdir/d.txt",  "subdir/d.txt",  Symlink, Some(FileMode(0o600)))
    ); "subdir_rule")]
    #[test_case(r#""**/*.txt" = { type = "copy" }"# => portal_entries!(
        ("a.txt",         "a.txt",         Copy, None),
        ("b.txt",         "b.txt",         Copy, None),
        ("subdir/c.txt",  "subdir/c.txt",  Copy, None),
        ("subdir/d.txt",  "subdir/d.txt",  Copy, None)
    ); "recursive_glob")]
    #[test_case(r#""*.rs" = { type = "copy" }"# => portal_entries!(
        ("a.txt",         "a.txt",         Symlink, None),
        ("b.txt",         "b.txt",         Symlink, None),
        ("subdir/c.txt",  "subdir/c.txt",  Symlink, None),
        ("subdir/d.txt",  "subdir/d.txt",  Symlink, None)
    ); "no_match")]
    fn test_apply_rules(rule: &str) -> HashMap<PathBuf, PortalEntry> {
        let (temp_dir, source_dir, target_dir) = setup_test(r#""" = """#, "", rule, true);
        let config = Config::read(&source_dir).unwrap();
        let ignore_matcher = build_ignore(&config.ignore, &target_dir).unwrap();
        let mut portal_entries =
            resolve_portals(&source_dir, &target_dir, &config.portal, &ignore_matcher).unwrap();
        apply_rules(&target_dir, &mut portal_entries, &config.rule).unwrap();
        flatten(portal_entries, temp_dir.path())
    }

    #[test_case(
        |s, t| {
            unix_fs::symlink(s.join("src"), t.join("target")).unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Symlink)
        => true; "symlink_identical")]
    #[test_case(
        |s, t| {
            unix_fs::symlink(s.join("other"), t.join("target")).unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Symlink)
        => false; "symlink_not_identical")]
    #[test_case(
        |s, _| {
            fs::write(s.join("src"), "").unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Symlink)
        => false; "symlink_target_not_symlink")]
    #[test_case(
        |s, t| {
            fs::write(s.join("src"), "a").unwrap();
            fs::write(t.join("target"), "a").unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => true; "copy_file_identical")]
    #[test_case(
        |s, t| {
            fs::write(s.join("src"), "a").unwrap();
            fs::write(t.join("target"), "b").unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => false; "copy_file_not_identical")]
    #[test_case(
        |s, t| {
            fs::write(s.join("src"), "a").unwrap();
            unix_fs::symlink(s.join("src"), t.join("target")).unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => false; "copy_file_target_is_symlink")]
    #[test_case(
        |s, _| {
            unix_fs::symlink(Path::new("/a"), s.join("src")).unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => false; "copy_symlink_source_target_missing")]
    #[test_case(
        |s, t| {
            unix_fs::symlink(Path::new("/a"), s.join("src")).unwrap();
            unix_fs::symlink(Path::new("/a"), t.join("target")).unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => true; "copy_symlink_source_identical")]
    #[test_case(
        |s, t| {
            unix_fs::symlink(Path::new("/a"), s.join("src")).unwrap();
            unix_fs::symlink(Path::new("/b"), t.join("target")).unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => false; "copy_symlink_source_not_identical")]
    #[test_case(
        |s, t| {
            unix_fs::symlink(Path::new("/a"), s.join("src")).unwrap();
            fs::write(t.join("target"), "").unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => false; "copy_symlink_source_target_is_file")]
    fn test_is_identical(
        setup: impl FnOnce(&Path, &Path),
        paths: impl FnOnce(&Path, &Path) -> (PathBuf, PathBuf, DeployType),
    ) -> bool {
        let temp_dir = tempdir().unwrap();
        let source_dir = temp_dir.path().join("source");
        let target_dir = temp_dir.path().join("target");
        fs::create_dir_all(&source_dir).unwrap();
        fs::create_dir_all(&target_dir).unwrap();

        setup(&source_dir, &target_dir);
        let (target, source, deploy_type) = paths(&source_dir, &target_dir);
        is_identical(&target, &source, deploy_type, &mut None, &mut None)
    }

    const FLAGS: ApplyFlags = ApplyFlags {
        dry_run: false,
        clean_up: false,
        prune_empty_dirs: false,
    };

    fn mock_apply(
        source_dir: &Path,
        target_dir: &Path,
        db_path: &Path,
        flags: ApplyFlags,
    ) -> Result<()> {
        run(
            GlobalFlags::new(
                Some(source_dir.to_path_buf()),
                Some(target_dir.to_path_buf()),
                None,
            ),
            db_path,
            flags,
        )
    }

    thread_local! {
        pub static CHECK_MANAGED: RefCell<bool> = const { RefCell::new(false) };
    }

    // --- Fresh deploy ---
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "").unwrap();
        },
        |s, t| {
            assert!(t.join("file").exists());
            assert!(t.join("file").is_symlink());
            assert_eq!(fs::read_link(t.join("file")).unwrap(), s.join("file"));
        },
        DeployType::Symlink; "symlink_fresh"
    )]
    #[test_case(
        |s, _| {
            fs::create_dir_all(s.join("dir")).unwrap();
            fs::write(s.join("dir/file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir/file").exists());
        },
        DeployType::Symlink; "symlink_nested"
    )]
    #[test_case(
        |s, _| {
            unix_fs::symlink(Path::new("/a"), s.join("file")).unwrap();
        },
        |s, t| {
            assert_eq!(fs::read_link(t.join("file")).unwrap(), s.join("file"));
        },
        DeployType::Symlink; "symlink_broken_source"
    )]
    #[test_case(
        |s, _| {
            unix_fs::symlink(Path::new("/a"), s.join("file")).unwrap();
        },
        |_, t| {
            assert_eq!(fs::read_link(t.join("file")).unwrap(), Path::new("/a"));
        },
        DeployType::Copy; "copy_symlink_source_fresh"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("file").exists());
            assert!(!t.join("file").is_symlink());
        },
        DeployType::Copy; "copy_fresh"
    )]
    #[test_case(
        |s, _| {
            fs::create_dir_all(s.join("dir/sub")).unwrap();
            fs::write(s.join("dir/file1"), "a").unwrap();
            fs::write(s.join("dir/sub/file2"), "b").unwrap();
        },
        |_, t| {
            assert_eq!(fs::read_to_string(t.join("dir/file1")).unwrap(), "a");
            assert_eq!(fs::read_to_string(t.join("dir/sub/file2")).unwrap(), "b");
        },
        DeployType::Copy; "copy_nested_dir_fresh"
    )]
    // --- Identical ---
    #[test_case(
        |s, t| {
            unix_fs::symlink(Path::new("/a"), s.join("file")).unwrap();
            unix_fs::symlink(s.join("file"), t.join("file")).unwrap();
        },
        |_, _| {
            assert!(!CHECK_MANAGED.with_borrow(|b| *b));
        },
        DeployType::Symlink; "symlink_identical"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("file"), "a").unwrap();
            fs::write(t.join("file"), "a").unwrap();
        },
        |_, _| {
            assert!(!CHECK_MANAGED.with_borrow(|b| *b));
        },
        DeployType::Copy; "copy_identical_file"
    )]
    #[test_case(
        |s, t| {
            unix_fs::symlink(Path::new("/a"), s.join("file")).unwrap();
            unix_fs::symlink(Path::new("/a"), t.join("file")).unwrap();
        },
        |_, _| {
            assert!(!CHECK_MANAGED.with_borrow(|b| *b));
        },
        DeployType::Copy; "copy_identical_symlink"
    )]
    // --- Collision: file vs dir ---
    #[test_case(
        |s, t| {
            fs::write(s.join("dir"), "").unwrap();
            fs::create_dir_all(t.join("dir")).unwrap();
            fs::write(t.join("dir/file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir/file").exists());
        },
        DeployType::Symlink; "symlink_file_vs_dir_skip"
    )]
    #[test_case(
        |s, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::write(s.join("dir"), "").unwrap();
            fs::create_dir_all(t.join("dir")).unwrap();
            fs::write(t.join("dir/file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir").is_file());
        },
        DeployType::Symlink; "symlink_file_vs_dir_overwrite"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("dir"), "").unwrap();
            fs::create_dir_all(t.join("dir")).unwrap();
            fs::write(t.join("dir/file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir/file").exists());
        },
        DeployType::Copy; "copy_file_vs_dir_skip"
    )]
    #[test_case(
        |s, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::write(s.join("dir"), "").unwrap();
            fs::create_dir_all(t.join("dir")).unwrap();
            fs::write(t.join("dir/file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir").is_file());
        },
        DeployType::Copy; "copy_file_vs_dir_overwrite"
    )]
    // --- Collision: dir vs file ---
    #[test_case(
        |s, t| {
            fs::create_dir_all(s.join("dir")).unwrap();
            fs::write(s.join("dir/file"), "").unwrap();
            fs::write(t.join("dir"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir").is_file());
        },
        DeployType::Symlink; "symlink_dir_vs_file_skip"
    )]
    #[test_case(
        |s, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::create_dir_all(s.join("dir")).unwrap();
            fs::write(s.join("dir/file"), "").unwrap();
            fs::write(t.join("dir"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir/file").exists());
        },
        DeployType::Symlink; "symlink_dir_vs_file_overwrite"
    )]
    #[test_case(
        |s, t| {
            fs::create_dir_all(s.join("dir")).unwrap();
            fs::write(s.join("dir/file"), "").unwrap();
            fs::write(t.join("dir"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir").is_file());
        },
        DeployType::Copy; "copy_dir_vs_file_skip"
    )]
    #[test_case(
        |s, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::create_dir_all(s.join("dir")).unwrap();
            fs::write(s.join("dir/file"), "").unwrap();
            fs::write(t.join("dir"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir/file").exists());
        },
        DeployType::Copy; "copy_dir_vs_file_overwrite"
    )]
    // --- Collision: file vs file ---
    #[test_case(
        |s, t| {
            fs::write(s.join("file"), "").unwrap();
            fs::write(t.join("file"), "").unwrap();
        },
        |_, t| {
            assert!(!t.join("file").is_symlink());
        },
        DeployType::Symlink; "symlink_file_vs_file_skip"
    )]
    #[test_case(
        |s, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::write(s.join("file"), "").unwrap();
            fs::write(t.join("file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("file").is_symlink());
        },
        DeployType::Symlink; "symlink_file_vs_file_overwrite"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("file"), "a").unwrap();
            fs::write(t.join("file"), "b").unwrap();
        },
        |_, t| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "b");
        },
        DeployType::Copy; "copy_file_vs_file_skip"
    )]
    #[test_case(
        |s, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::write(s.join("file"), "a").unwrap();
            fs::write(t.join("file"), "b").unwrap();
        },
        |_, t| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "a");
        },
        DeployType::Copy; "copy_file_vs_file_overwrite"
    )]
    // --- Collision: quit ---
    #[test_case(
        |s, t| {
            fs::write(s.join("file1"), "").unwrap();
            PROMPT_SELECTION.set(CollisionOptions::Quit);
            fs::write(s.join("file2"), "").unwrap();
            fs::write(t.join("file2"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("file1").exists());
            assert!(!t.join("file2").exists());
        },
        DeployType::Symlink => panics "Abort"; "quit_symlink"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("file1"), "").unwrap();
            PROMPT_SELECTION.set(CollisionOptions::Quit);
            fs::write(s.join("file"), "a").unwrap();
            fs::write(t.join("file"), "b").unwrap();
        },
        |_, t| {
            assert!(t.join("file1").exists());
            assert!(!t.join("file2").exists());
        },
        DeployType::Copy => panics "Abort"; "quit_copy"
    )]
    // --- Symlink to dir ---
    #[test_case(
        |s, t| {
            fs::create_dir_all(t.join("real_dir")).unwrap();
            unix_fs::symlink(t.join("real_dir"), s.join("link_dir")).unwrap();
        },
        |s, t| {
            assert!(t.join("link_dir").is_symlink());
            assert_eq!(fs::read_link(t.join("link_dir")).unwrap(), s.join("link_dir"));
        },
        DeployType::Symlink; "symlink_to_dir_as_source"
    )]
    #[test_case(
        |s, t| {
            fs::create_dir_all(t.join("real_dir")).unwrap();
            unix_fs::symlink(t.join("real_dir"), s.join("link_dir")).unwrap();
        },
        |_, t| {
            assert!(t.join("link_dir").is_symlink());
            assert_eq!(fs::read_link(t.join("link_dir")).unwrap(), t.join("real_dir"));
        },
        DeployType::Copy; "copy_symlink_to_dir_as_source"
    )]
    fn test_apply(
        setup: impl FnOnce(&Path, &Path),
        assert: impl FnOnce(&Path, &Path),
        deploy_type: DeployType,
    ) {
        let (temp_dir, source_dir, target_dir) = setup_test(
            r#""" = """#,
            "",
            match deploy_type {
                DeployType::Symlink => "",
                DeployType::Copy => r#""**/*" = { type = "copy" }"#,
            },
            false,
        );
        setup(&source_dir, &target_dir);
        mock_apply(&source_dir, &target_dir, &temp_dir.path().join("db"), FLAGS).unwrap();
        assert(&source_dir, &target_dir);
    }

    // --- Re-apply identical (no-op) ---
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, _| {},
        |s, t, db| {
            assert!(t.join("file").exists());
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert_eq!(entry.hash.unwrap(), hash_file(&s.join("file")).unwrap());
        },
        DeployType::Copy; "copy_reapply_identical"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, _| {},
        |s, t, db| {
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Symlink);
            assert_eq!(fs::read_link(t.join("file")).unwrap(), s.join("file"));
            assert!(entry.hash.is_none());
        },
        DeployType::Symlink; "symlink_reapply_identical"
    )]
    // --- Re-apply after source changed ---
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |s, _| {
            fs::write(s.join("file"), "b").unwrap();
        },
        |s, t, db| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "b");
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert_eq!(entry.hash.unwrap(), hash_file(&s.join("file")).unwrap());
        },
        DeployType::Copy; "copy_reapply_source_changed"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |s, _| {
            fs::write(s.join("file"), "b").unwrap();
        },
        |s, t, db| {
            assert_eq!(fs::read_link(t.join("file")).unwrap(), s.join("file"));
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Symlink);
            assert!(entry.hash.is_none());
        },
        DeployType::Symlink; "symlink_reapply_source_changed"
    )]
    // --- Re-apply after externally modified target (unmanaged) ---
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, t| {
            fs::write(t.join("file"), "external").unwrap();
        },
        |s, t, db| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "external");
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert_eq!(entry.hash.unwrap(), hash_file(&s.join("file")).unwrap());
        },
        DeployType::Copy; "copy_reapply_external_modification_skip"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::write(t.join("file"), "external").unwrap();
        },
        |s, t, db| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "a");
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert_eq!(entry.hash.unwrap(), hash_file(&s.join("file")).unwrap());
        },
        DeployType::Copy; "copy_reapply_external_modification_overwrite"
    )]
    // --- Unmanaged symlink target on re-apply ---
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, t| {
            let _ = fs::remove_file(t.join("file"));
            fs::write(t.join("file"), "external").unwrap();
        },
        |_, t, db| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "external");
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Symlink);
            assert!(entry.hash.is_none());
        },
        DeployType::Symlink; "symlink_reapply_target_replaced_with_file_skip"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, t| {
            let _ = fs::remove_file(t.join("file"));
            unix_fs::symlink(Path::new("/wrong"), t.join("file")).unwrap();
        },
        |s, t, db| {
            assert_eq!(fs::read_link(t.join("file")).unwrap(), Path::new("/wrong"));
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.source_path, s.join("file"));
            assert_eq!(entry.deploy_type, DeployType::Symlink);
            assert!(entry.hash.is_none());
        },
        DeployType::Symlink; "symlink_reapply_target_replaced_with_wrong_symlink_skip"
    )]
    // --- Copy with symlink source on re-apply ---
    #[test_case(
        |s, _| {
            unix_fs::symlink(Path::new("/a"), s.join("file")).unwrap();
        },
        |_, _| {},
        |_, t, db| {
            assert!(t.join("file").is_symlink());
            assert_eq!(fs::read_link(t.join("file")).unwrap(), Path::new("/a"));
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert!(entry.hash.is_none());
            assert_eq!(entry.symlink_target, Some(PathBuf::from("/a")));
        },
        DeployType::Copy; "copy_reapply_symlink_source_identical"
    )]
    #[test_case(
        |s, _| {
            unix_fs::symlink(Path::new("/a"), s.join("file")).unwrap();
        },
        |s, _| {
            let _ = fs::remove_file(s.join("file"));
            unix_fs::symlink(Path::new("/b"), s.join("file")).unwrap();
        },
        |_, t, db| {
            assert!(t.join("file").is_symlink());
            assert_eq!(fs::read_link(t.join("file")).unwrap(), Path::new("/b"));
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert!(entry.hash.is_none());
            assert_eq!(entry.symlink_target, Some(PathBuf::from("/b")));
        },
        DeployType::Copy; "copy_reapply_symlink_source_changed"
    )]
    // --- Target type changed externally (skip/overwrite) ---
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, t| {
            let _ = fs::remove_file(t.join("file"));
            unix_fs::symlink(Path::new("/evil"), t.join("file")).unwrap();
        },
        |s, t, db| {
            assert!(t.join("file").is_symlink());
            assert_eq!(fs::read_link(t.join("file")).unwrap(), Path::new("/evil"));
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert_eq!(entry.hash.unwrap(), hash_file(&s.join("file")).unwrap());
        },
        DeployType::Copy; "copy_reapply_target_type_changed_skip"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            let _ = fs::remove_file(t.join("file"));
            unix_fs::symlink(Path::new("/evil"), t.join("file")).unwrap();
        },
        |_, t, db| {
            assert!(!t.join("file").is_symlink());
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "a");
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert!(entry.hash.is_some());
        },
        DeployType::Copy; "copy_reapply_target_type_changed_overwrite"
    )]
    fn test_apply_twice(
        setup1: impl FnOnce(&Path, &Path),
        setup2: impl FnOnce(&Path, &Path),
        assert: impl FnOnce(&Path, &Path, &Db),
        deploy_type: DeployType,
    ) {
        let (temp_dir, source_dir, target_dir) = setup_test(
            r#""" = """#,
            "",
            match deploy_type {
                DeployType::Symlink => "",
                DeployType::Copy => r#""**/*" = { type = "copy" }"#,
            },
            false,
        );
        setup1(&source_dir, &target_dir);
        let db_path = temp_dir.path().join("db");
        mock_apply(&source_dir, &target_dir, &db_path, FLAGS).unwrap();
        setup2(&source_dir, &target_dir);
        mock_apply(&source_dir, &target_dir, &db_path, FLAGS).unwrap();
        let db = Db::init(&db_path).unwrap();
        assert(&source_dir, &target_dir, &db);
    }

    #[test]
    fn test_deploy_permission() {
        let (temp_dir, source_dir, target_dir) = setup_test(
            r#""" = """#,
            "",
            r#""**/*" = { type = "copy", mode = "123" }"#,
            false,
        );
        fs::write(source_dir.join("file"), "").unwrap();
        mock_apply(&source_dir, &target_dir, &temp_dir.path().join("db"), FLAGS).unwrap();
        assert_eq!(
            target_dir
                .join("file")
                .metadata()
                .unwrap()
                .permissions()
                .mode(),
            0o100123
        );
    }

    #[test_case(FLAGS; "creates_file")]
    #[test_case(ApplyFlags { dry_run: true, ..FLAGS }; "dry_run_no_file")]
    fn test_apply_deploy(flags: ApplyFlags) {
        let (temp_dir, source_dir, target_dir) = setup_test(r#""" = """#, "", "", false);
        fs::write(source_dir.join("file"), "").unwrap();
        mock_apply(&source_dir, &target_dir, &temp_dir.path().join("db"), flags).unwrap();
        assert_eq!(target_dir.join("file").exists(), !flags.dry_run);
    }

    #[test_case(false; "removes_file")]
    #[test_case(true; "dry_run_preserves_file")]
    fn test_apply_clean_up(dry_run: bool) {
        let (temp_dir, source_dir, target_dir) = setup_test(r#""" = """#, "", "", false);
        fs::write(source_dir.join("file"), "").unwrap();
        mock_apply(&source_dir, &target_dir, &temp_dir.path().join("db"), FLAGS).unwrap();
        assert!(target_dir.join("file").exists());
        fs::write(source_dir.join("dotrift.toml"), "").unwrap();
        mock_apply(
            &source_dir,
            &target_dir,
            &temp_dir.path().join("db"),
            ApplyFlags {
                dry_run,
                clean_up: true,
                ..FLAGS
            },
        )
        .unwrap();
        assert_eq!(target_dir.join("file").exists(), dry_run);
    }
}
