use std::{
    collections::HashMap,
    fs::{self, remove_dir_all, remove_file},
    path::{Path, PathBuf},
};

use color_eyre::eyre::{Context, Result, eyre};
use glob::Pattern;
use normalize_path::NormalizePath;
use walkdir::WalkDir;

use crate::{
    cli::{AddFlags, GlobalFlags, OpenEditor},
    command::{
        apply::{build_ignore, is_ignored},
        to_absolute_path, tree,
        util::{
            GLOB_OPTION, PathLiteral, SafeStripPrefix, copy_recursive, is_glob, resolve_target,
            strip_prefix_filter_glob,
        },
    },
    config::Config,
    db::Db,
    output,
};

pub fn run(
    global_flags: GlobalFlags,
    path: PathBuf,
    destination: Option<PathBuf>,
    flags: AddFlags,
    db_path: &Path,
) -> Result<()> {
    let source_dir = global_flags.source()?;
    let config_override = global_flags.config()?;
    let path = to_absolute_path(&path)?;

    let reimport = destination.is_none();

    let reimport_dir = path.path_is_dir() && reimport;

    let entries: Vec<(PathBuf, PathBuf)> = if reimport_dir {
        let db = Db::init(db_path)?;
        let mut entries = Vec::new();
        for entry in WalkDir::new(&path)
            .into_iter()
            .flatten()
            .filter(|e| !e.file_type().is_dir())
        {
            let p = entry.path();
            match db.get_entry(p)? {
                Some(db_entry) => {
                    let dest = db_entry.source_path;
                    if !dest.starts_with(&source_dir) {
                        output::print_warn(format!(
                            "destination `{}` is outside source directory, skipping",
                            p.display()
                        ));
                        continue;
                    }
                    if dest.path_exists() && !flags.force {
                        output::print_warn(format!(
                            "`{}` already exists, skipping",
                            dest.display()
                        ));
                        continue;
                    }
                    entries.push((p.to_path_buf(), dest));
                }
                None => {
                    output::print_warn(format!("`{}` not in database, skipping", p.display()));
                }
            }
        }
        entries
    } else {
        let dest = if let Some(d) = destination {
            if d.is_absolute() {
                d
            } else {
                source_dir.join(d).normalize()
            }
        } else {
            let db = Db::init(db_path)?;
            match db.get_entry(&path)? {
                Some(entry) => entry.source_path,
                None => {
                    return Err(eyre!("Path `{}` not found in database", path.display()));
                }
            }
        };
        if !dest.starts_with(&source_dir) {
            return Err(eyre!(
                "Destination '{}' must be inside source directory '{}'",
                dest.display(),
                source_dir.display()
            ));
        }
        vec![(path, dest)]
    };

    if !reimport_dir {
        let dest = &entries[0].1;
        if dest.path_exists() && !flags.force {
            return Err(eyre!("`{}` already exists", dest.display()));
        }
    }

    for (src, dest) in &entries {
        if flags.force {
            remove_obstructions(dest)?;
        }
        if let Some(parent) = dest.parent() {
            crate::create_dir_err!(fs::create_dir_all(parent), parent)?;
        }
        if reimport || flags.copy {
            copy_recursive(src, dest)
        } else {
            fs::rename(src, dest).wrap_err_with(|| {
                format!("Failed to move `{}` to `{}`", src.display(), dest.display())
            })
        }?;
    }

    let config = Config::read(&source_dir)?;
    let target_override = global_flags.target()?;
    let target_dir = resolve_target(&source_dir, target_override, &config)?;

    let analysis = analyze_portal(&config, &entries, &target_dir, &source_dir)?;

    let should_open = match flags.editor {
        Some(OpenEditor::Always) => true,
        Some(OpenEditor::Never) => false,
        None => !analysis.missing.is_empty() || !analysis.collisions.is_empty(),
    };

    #[cfg(test)]
    {
        tests::OPEN_EDITOR.set(should_open);
    }

    if should_open {
        launch_editor(&source_dir, config_override)?;
    }

    Ok(())
}

fn portal_matches(dest_rel: &Path, portal: &HashMap<String, PathBuf>) -> bool {
    portal.keys().any(|pattern| {
        if is_glob(pattern) {
            Pattern::new(pattern)
                .ok()
                .map(|g| g.matches_path_with(dest_rel, GLOB_OPTION))
                .unwrap_or(false)
        } else {
            let path = Path::new(pattern);
            dest_rel.ancestors().any(|a| a == path)
        }
    })
}

pub struct PortalAnalysis {
    /// (dest_rel, computed_target) needing auto-add in step 8
    pub missing: Vec<(PathBuf, PathBuf)>,
    /// (collision_id, [portal_keys]) — keys to annotate with # CONFLICT <id>
    pub collisions: Vec<(usize, Vec<String>)>,
}

fn resolve_collisions(
    portal: &HashMap<String, PathBuf>,
    ignore: &[String],
    target_dir: &Path,
    source_dir: &Path,
) -> Result<(tree::Node, HashMap<String, Vec<String>>)> {
    let ignore_matcher = build_ignore(ignore, target_dir)?;
    let mut root = tree::Node::default();
    let mut collisions: HashMap<String, Vec<String>> = HashMap::new();

    for (pattern, target_rel) in portal {
        let pattern_normalized = Path::new(pattern).normalize();
        let pattern_str = pattern_normalized.to_string_lossy().to_string();
        let target_rel_normalized = target_rel.normalize();

        if is_glob(&pattern_str) {
            let prefix = strip_prefix_filter_glob(&pattern_str);
            let full_pattern = source_dir.join(&pattern_str);
            let full_pattern_str = full_pattern.to_string_lossy();

            let Ok(paths) = crate::glob_err!(
                glob::glob_with(&full_pattern_str, GLOB_OPTION),
                &full_pattern_str
            ) else {
                continue;
            };
            for source_path in paths.flatten() {
                if source_path.path_is_dir() {
                    continue;
                }
                let source_rel = source_path.safe_strip_prefix(source_dir);
                let stripped = if prefix.is_empty() {
                    source_rel.to_path_buf()
                } else {
                    source_rel.safe_strip_prefix(&prefix).to_path_buf()
                };
                let target_path = target_dir.join(&target_rel_normalized).join(stripped);
                if is_ignored(&ignore_matcher, &target_path) {
                    continue;
                }
                {
                    match root.check_entry(&target_path, pattern_str.clone()) {
                        Ok(Some(existing)) => {
                            let t = target_path.display().to_string();
                            collisions.entry(t.clone()).or_default().push(existing);
                            collisions.entry(t).or_default().push(pattern_str.clone());
                        }
                        Err(_) => {
                            collisions
                                .entry(target_path.display().to_string())
                                .or_default()
                                .push(pattern_str.clone());
                        }
                        _ => {}
                    }
                }
            }
        } else {
            let source_path = source_dir.join(&pattern_normalized);
            if !source_path.path_exists() {
                continue;
            }

            if source_path.path_is_dir() {
                let walker = WalkDir::new(&source_path)
                    .into_iter()
                    .flatten()
                    .filter(|e| !e.file_type().is_dir());
                for entry in walker {
                    let file_source = entry.path().to_path_buf();
                    let rel_to_pattern = file_source.safe_strip_prefix(&source_path);
                    let target_path = target_dir.join(&target_rel_normalized).join(rel_to_pattern);
                    if is_ignored(&ignore_matcher, &target_path) {
                        continue;
                    }
                    {
                        match root.check_entry(&target_path, pattern_str.clone()) {
                            Ok(Some(existing)) => {
                                let t = target_path.display().to_string();
                                collisions.entry(t.clone()).or_default().push(existing);
                                collisions.entry(t).or_default().push(pattern_str.clone());
                            }
                            Err(_) => {
                                collisions
                                    .entry(target_path.display().to_string())
                                    .or_default()
                                    .push(pattern_str.clone());
                            }
                            _ => {}
                        }
                    }
                }
            } else {
                let target_path = target_dir.join(&target_rel_normalized);
                if is_ignored(&ignore_matcher, &target_path) {
                    continue;
                }
                {
                    match root.check_entry(&target_path, pattern_str.clone()) {
                        Ok(Some(existing)) => {
                            let t = target_path.display().to_string();
                            collisions.entry(t.clone()).or_default().push(existing);
                            collisions.entry(t).or_default().push(pattern_str.clone());
                        }
                        Err(_) => {
                            collisions
                                .entry(target_path.display().to_string())
                                .or_default()
                                .push(pattern_str.clone());
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    Ok((root, collisions))
}

fn check_new_entries(
    tree: &mut tree::Node,
    entries: &[(PathBuf, PathBuf)],
    target_dir: &Path,
    source_dir: &Path,
) -> HashMap<String, Vec<String>> {
    let mut new_collisions: HashMap<String, Vec<String>> = HashMap::new();

    for (src, dest) in entries {
        let computed_target = if src.starts_with(target_dir) {
            src.safe_strip_prefix(target_dir).to_path_buf()
        } else {
            src.clone()
        };
        let dest_rel = dest.safe_strip_prefix(source_dir);

        let target_path = target_dir.join(&computed_target).normalize();
        let key = dest_rel.display().to_string();
        match tree.check_entry(&target_path, key.clone()) {
            Ok(Some(existing)) => {
                let t = target_path.display().to_string();
                new_collisions.entry(t.clone()).or_default().push(existing);
                new_collisions.entry(t).or_default().push(key);
            }
            Err(e) => {
                println!("{e}");
                new_collisions
                    .entry(target_path.display().to_string())
                    .or_default()
                    .push(key);
            }
            _ => {}
        }
    }

    new_collisions
}

fn merge_collision_groups(
    existing: HashMap<String, Vec<String>>,
    new: HashMap<String, Vec<String>>,
) -> Vec<(usize, Vec<String>)> {
    let mut merged: HashMap<String, Vec<String>> = existing;
    for (target, keys) in new {
        merged.entry(target).or_default().extend(keys);
    }

    let mut groups: Vec<Vec<String>> = Vec::new();
    for keys in merged.into_values() {
        let mut uniq: Vec<String> = Vec::new();
        for k in keys {
            if !uniq.contains(&k) {
                uniq.push(k);
            }
        }
        uniq.sort();
        groups.push(uniq);
    }

    groups
        .into_iter()
        .enumerate()
        .map(|(i, keys)| (i + 1, keys))
        .collect()
}

pub fn analyze_portal(
    config: &Config,
    entries: &[(PathBuf, PathBuf)],
    target_dir: &Path,
    source_dir: &Path,
) -> Result<PortalAnalysis> {
    let (mut tree, existing_collisions) =
        resolve_collisions(&config.portal, &config.ignore, target_dir, source_dir)?;

    let new_collisions = check_new_entries(&mut tree, entries, target_dir, source_dir);

    let collisions = merge_collision_groups(existing_collisions, new_collisions);

    let mut missing = Vec::new();
    for (src, dest) in entries {
        let computed_target = if src.starts_with(target_dir) {
            src.safe_strip_prefix(target_dir).to_path_buf()
        } else {
            src.clone()
        };
        let dest_rel = dest.safe_strip_prefix(source_dir).to_path_buf();

        if !portal_matches(&dest_rel, &config.portal) {
            missing.push((dest_rel, computed_target));
        }
    }

    Ok(PortalAnalysis {
        missing,
        collisions,
    })
}

#[cfg(test)]
fn launch_editor(_: &Path, _: Option<PathBuf>) -> Result<()> {
    Ok(())
}

#[cfg(not(test))]
fn launch_editor(source_dir: &Path, config_override: Option<PathBuf>) -> Result<()> {
    use crate::{
        global_config::{GlobalConfig, expand_args, find_portal_cursor},
        path::config_path,
    };
    use color_eyre::Section;
    use std::{env, process::Command};

    let file = config_path(source_dir).to_string_lossy().to_string();
    let (row, col) = if let Ok(content) = fs::read_to_string(&file) {
        find_portal_cursor(&content)
    } else {
        (1, 1)
    };

    let (cmd, args) = match GlobalConfig::read(config_override)? {
        GlobalConfig {
            editor_command: Some(config),
            ..
        } => {
            let params = [
                ("file", file.as_str()),
                ("row", &row.to_string()),
                ("col", &col.to_string()),
            ];
            let args = expand_args(&config.args, &params)
                .wrap_err("Failed to expand editor command parameters")?;
            (config.command, args)
        }
        _ => match env::var_os("VISUAL").or(env::var_os("EDITOR")) {
            Some(cmd) => (cmd.to_string_lossy().to_string(), vec![file]),
            None => {
                let config_path = crate::path::global_config_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "$XDG_CONFIG_HOME/dotrift/config.toml".into());
                return Err(eyre!("No editor found").suggestion(format!(
                    "Set $VISUAL or $EDITOR, or configure editor-command in {config_path}"
                )));
            }
        },
    };

    // TODO check command exist
    Command::new(&cmd).args(&args).status().wrap_err_with(|| {
        format!(
            "Failed to launch editor: {}",
            [vec![cmd], args].concat().join(" ")
        )
    })?;
    Ok(())
}

fn remove_obstructions(path: &Path) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.is_dir() {
            crate::remove_dir_err!(remove_dir_all(path), path)?;
        } else {
            crate::remove_file_err!(remove_file(path), path)?;
        }
    }
    for dir in path.ancestors().skip(1) {
        let Ok(meta) = fs::symlink_metadata(dir) else {
            continue;
        };
        if meta.is_dir() {
            break;
        }
        crate::remove_file_err!(remove_file(dir), dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, os::unix::fs as unix_fs, path::Path};

    use crate::{command::util::tests::setup_test, config::DeployType, db::DbEntry};

    use super::*;
    use test_case::test_case;

    const FLAGS: AddFlags = AddFlags {
        copy: false,
        force: false,
        editor: None,
        no_modify: false,
    };

    fn mock_add(source_dir: &Path, path: &Path, destination: &Path, flags: AddFlags) {
        let temp_db = tempfile::tempdir().unwrap();
        run(
            GlobalFlags::new(Some(source_dir.to_path_buf()), None, None),
            path.to_path_buf(),
            Some(destination.to_path_buf()),
            flags,
            &temp_db.path().join("db"),
        )
        .unwrap();
    }

    fn mock_add_reimport(source_dir: &Path, path: &Path, flags: AddFlags, db_path: &Path) {
        run(
            GlobalFlags::new(Some(source_dir.to_path_buf()), None, None),
            path.to_path_buf(),
            None,
            flags,
            db_path,
        )
        .unwrap();
    }

    // --- simple file moves (destination path variations) ---
    #[test_case(
        |t, _| {
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), "file".into())
        },
        |s, t| {
            assert!(!t.join("file").exists());
            assert!(s.join("file").exists());
        }; "move_relative_path"
    )]
    #[test_case(
        |t, s| {
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), s.join("file"))
        },
        |s, t| {
            assert!(!t.join("file").exists());
            assert!(s.join("file").exists());
        }; "move_absolute_path"
    )]
    #[test_case(
        |t, _| {
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), "./dir/../file".into())
        },
        |s, _| {
            assert!(s.join("file").exists());
        }; "move_normalized_path"
    )]
    // --- source type variations ---
    #[test_case(
        |t, s| {
            fs::write(t.join("real"), "data").unwrap();
            unix_fs::symlink(t.join("real"), t.join("link")).unwrap();
            (t.join("link"), s.join("link"))
        },
        |s, t| {
            assert!(s.join("link").is_symlink());
            assert_eq!(
                fs::read_link(s.join("link")).unwrap(),
                t.join("real")
            );
        }; "move_symlink"
    )]
    #[test_case(
        |t, s| {
            fs::create_dir_all(t.join("dir/sub")).unwrap();
            fs::write(t.join("dir/sub/file"), "data").unwrap();
            (t.join("dir"), s.join("dir"))
        },
        |s, _| {
            assert!(s.join("dir").is_dir());
            assert!(s.join("dir/sub/file").exists());
        }; "move_directory"
    )]
    // --- destination path depth ---
    #[test_case(
        |t, s| {
            fs::write(t.join("file"), "data").unwrap();
            (t.join("file"), s.join("a/b/c/file"))
        },
        |s, _| {
            assert!(s.join("a/b/c/file").exists());
        }; "move_to_nested_dest"
    )]
    // --- failure cases ---
    #[test_case(
        |_, s| {
            (s.join("nonexistent"), s.join("dest"))
        },
        |_, _| {} => panics "move"; "fail_missing_source"
    )]
    #[test_case(
        |t, s| {
            fs::write(t.join("file"), "").unwrap();
            fs::write(s.join("file"), "").unwrap();
            (t.join("file"), s.join("file"))
        },
        |_, _| {} => panics "already exists"; "fail_dest_exists"
    )]
    #[test_case(
        |t, _| {
            (t.join("file"), "../file".into())
        },
        |_, _| {} => panics "inside"; "fail_escape_source"
    )]
    fn test_add_move(
        setup: impl FnOnce(&Path, &Path) -> (PathBuf, PathBuf),
        assertion: impl FnOnce(&Path, &Path),
    ) {
        let (temp_dir, source_dir, _) = setup_test("", "", "", false);
        let (f, d) = setup(temp_dir.path(), &source_dir);

        mock_add(&source_dir, &f, &d, FLAGS);

        assertion(&source_dir, temp_dir.path());
    }

    #[test]
    fn test_add_copy() {
        let (temp_dir, source_dir, _) = setup_test("", "", "", false);

        let f = temp_dir.path().join("file");
        let d = source_dir.join("file");
        fs::write(&f, "").unwrap();
        mock_add(
            &source_dir,
            &f,
            &d,
            AddFlags {
                copy: true,
                ..FLAGS
            },
        );
        assert!(f.exists());
        assert!(d.exists());
    }

    #[test]
    fn test_add_force() {
        let (temp_dir, source_dir, _) = setup_test("", "", "", false);

        let f = temp_dir.path().join("file");
        let d = source_dir.join("file");
        fs::write(&f, "a").unwrap();
        fs::write(&d, "b").unwrap();
        mock_add(
            &source_dir,
            &f,
            &d,
            AddFlags {
                force: true,
                ..FLAGS
            },
        );
        assert_eq!(fs::read_to_string(d).unwrap(), "a");
    }

    #[test_case(
        |t, s| {
            fs::write(t.join("file"), "data").unwrap();
            (t.join("file"), vec![DbEntry {
                target_path: t.join("file"),
                deploy_type: DeployType::Copy,
                source_path: s.join("file"),
                hash: None,
                symlink_target: None,
            }])
        },
        |s| {
            assert_eq!(fs::read_to_string(s.join("file")).unwrap(), "data");
        }; "reimport_file"
    )]
    #[test_case(
        |t, s| {
            fs::write(t.join("real"), "data").unwrap();
            unix_fs::symlink(t.join("real"), t.join("link")).unwrap();
            (t.join("link"), vec![DbEntry {
                target_path: t.join("link"),
                deploy_type: DeployType::Copy,
                source_path: s.join("link"),
                hash: None,
                symlink_target: Some(t.join("real")),
            }])
        },
        |s| {
            assert!(s.join("link").is_symlink());
        }; "reimport_symlink"
    )]
    #[test_case(
        |t, s| {
            fs::create_dir_all(t.join("dir/sub")).unwrap();
            fs::write(t.join("dir/file1"), "a").unwrap();
            fs::write(t.join("dir/sub/file2"), "b").unwrap();
            (t.join("dir"), vec![
                DbEntry {
                    target_path: t.join("dir/file1"),
                    deploy_type: DeployType::Copy,
                    source_path: s.join("dest/file1"),
                    hash: None,
                    symlink_target: None,
                },
                DbEntry {
                    target_path: t.join("dir/sub/file2"),
                    deploy_type: DeployType::Copy,
                    source_path: s.join("dest/sub/file2"),
                    hash: None,
                    symlink_target: None,
                },
            ])
        },
        |s| {
            assert_eq!(fs::read_to_string(s.join("dest/file1")).unwrap(), "a");
            assert_eq!(fs::read_to_string(s.join("dest/sub/file2")).unwrap(), "b");
        }; "reimport_directory"
    )]
    #[test_case(
        |t, s| {
            fs::create_dir_all(t.join("dir")).unwrap();
            fs::write(t.join("dir/file1"), "").unwrap();
            fs::write(t.join("dir/file2"), "").unwrap();
            (t.join("dir"), vec![
                DbEntry {
                    target_path: t.join("dir/file1"),
                    deploy_type: DeployType::Copy,
                    source_path: s.join("file"),
                    hash: None,
                    symlink_target: None,
                },
            ])
        },
        |s| {
            assert!(!s.join("dir").exists());
            assert!(s.join("file").exists());
        }; "reimport_directory_partial"
    )]
    fn test_add_reimport(
        setup: impl FnOnce(&Path, &Path) -> (PathBuf, Vec<DbEntry>),
        assertion: impl FnOnce(&Path),
    ) {
        let (temp_dir, source_dir, _) = setup_test("", "", "", false);
        let db_path = &temp_dir.path().join("db");
        let (p, es) = setup(temp_dir.path(), &source_dir);

        let db = Db::init(db_path).unwrap();
        for e in es {
            db.insert_or_update(&e).unwrap();
        }
        mock_add_reimport(&source_dir, &p, FLAGS, db_path);
        assertion(&source_dir);
    }

    #[test_case(
        |t| {
            fs::write(t.join("x"), "data").unwrap();
            t.join("x")
        },
        |t| {
            assert!(!t.join("x").exists());
        }
        ; "file_at_dest"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("x")).unwrap();
            t.join("x")
        },
        |t| {
            assert!(!t.join("x").exists());
        }
        ; "empty_dir_at_dest"
    )]
    #[test_case(
        |t| {
            let x = t.join("x");
            fs::create_dir_all(x.join("sub/nested")).unwrap();
            fs::write(x.join("sub/nested/f"), "data").unwrap();
            x
        },
        |t| {
            assert!(!t.join("x").exists());
        }
        ; "nonempty_dir_at_dest"
    )]
    #[test_case(
        |t| {
            unix_fs::symlink(Path::new("/nonexistent"), t.join("link")).unwrap();
            t.join("link")
        },
        |t| {
            assert!(!t.join("link").is_symlink());
        }
        ; "dangling_symlink_at_dest"
    )]
    #[test_case(
        |t| {
            let real = t.join("real");
            fs::write(&real, "data").unwrap();
            unix_fs::symlink(&real, t.join("link")).unwrap();
            t.join("link")
        },
        |t| {
            assert!(!t.join("link").is_symlink());
            assert!(t.join("real").exists());
        }
        ; "valid_symlink_file_at_dest"
    )]
    #[test_case(
        |t| {
            let real = t.join("dir");
            fs::create_dir(&real).unwrap();
            fs::write(real.join("f"), "data").unwrap();
            unix_fs::symlink(&real, t.join("link")).unwrap();
            t.join("link")
        },
        |t| {
            assert!(!t.join("link").is_symlink());
            assert!(t.join("dir/f").exists());
        }
        ; "symlink_to_dir_at_dest"
    )]
    #[test_case(
        |t| {
            t.join("ghost")
        },
        |_| {}
        ; "nonexistent_dest"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("a"), "block").unwrap();
            t.join("a/b/c")
        },
        |t| {
            assert!(!t.join("a").exists());
        }
        ; "file_blocking_parent"
    )]
    #[test_case(
        |t| {
            unix_fs::symlink(Path::new("/nonexistent"), t.join("a")).unwrap();
            t.join("a/b/c")
        },
        |t| {
            assert!(!t.join("a").is_symlink());
        }
        ; "dangling_symlink_blocking_parent"
    )]
    #[test_case(
        |t| {
            let real = t.join("real");
            fs::write(&real, "data").unwrap();
            unix_fs::symlink(&real, t.join("a")).unwrap();
            t.join("a/b/c")
        },
        |t| {
            assert!(!t.join("a").is_symlink());
            assert!(t.join("real").exists());
        }
        ; "valid_symlink_blocking_parent"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("dir")).unwrap();
            t.join("dir/target")
        },
        |t| {
            assert!(t.join("dir").is_dir());
        }
        ; "no_obstruction_existing_parent"
    )]
    #[test_case(
        |t| {
            t.join("a/b/c")
        },
        |_| {}
        ; "no_obstruction_no_parent"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("x"), "dest_content").unwrap();
            t.join("x")
        },
        |t| {
            assert!(!t.join("x").exists());
        }
        ; "file_at_dest_clean_parent"
    )]
    fn test_remove_obstructions(setup: impl FnOnce(&Path) -> PathBuf, assert: impl FnOnce(&Path)) {
        let t = tempfile::tempdir().unwrap();
        let target = setup(t.path());
        remove_obstructions(&target).unwrap();
        assert(t.path());
    }

    thread_local! {
        pub static OPEN_EDITOR: RefCell<bool> = const { RefCell::new(false) };
    }

    #[test_case("", "file" => true; "empty")]
    #[test_case(r#""" = """#, "file" => false; "match_root")]
    #[test_case(r#""dir/file" = """#, "dir/file" => false; "match_exact")]
    #[test_case(r#""dir" = """#, "dir/file" => false; "match_parent")]
    #[test_case(r#""a" = """#, "a/b/c" => false; "match_ancestor")]
    #[test_case(r#""a/b" = """#, "a/b/c" => false; "match_nested")]
    #[test_case(r#""a/b/c" = """#, "a/b/c" => false; "match_deep")]
    #[test_case(r#""*" = ".""#, "file" => false; "match_glob")]
    #[test_case(r#""dir/*" = """#, "dir/file" => false; "match_glob_parent")]
    #[test_case(r#""**/file" = """#, "a/b/file" => false; "match_glob_recursive")]
    #[test_case(r#""file1" = ""
"file2" = """#, "file1" => false; "match_first_portal")]
    #[test_case(r#""file1" = ""
"file2" = """#, "file2" => false; "match_second_portal")]
    #[test_case(r#""*.txt" = """#, "file" => true; "mismatch_glob")]
    #[test_case(r#""*.txt" = """#, "file.cfg" => true; "mismatch_glob_extension")]
    #[test_case(r#""file1" = """#, "file2" => true; "mismatch_literal")]
    #[test_case(r#""file" = """#, "file2" => true; "mismatch_prefix")]
    fn test_portal_matches(portal: &str, dest: &str) -> bool {
        let (_temp_dir, source_dir, _) = setup_test(portal, "", "", false);
        let config = Config::read(&source_dir).unwrap();

        !portal_matches(Path::new(dest), &config.portal)
    }

    #[test_case(
        r#""a" = "x""#,
        &["a"],
        ""
        => Vec::<(String, Vec<String>)>::new()
        ; "no_collision"
    )]
    #[test_case(
        r#""a" = "x"
"b" = "x""#,
        &["a", "b"],
        ""
        => vec![("x".into(), vec!["a".into(), "b".into()])]
        ; "same_target_two_literals"
    )]
    #[test_case(
        r#""subdir" = "out""#,
        &["subdir/f1", "subdir/f2"],
        ""
        => Vec::<(String, Vec<String>)>::new()
        ; "literal_dir"
    )]
    #[test_case(
        r#""subdir" = "out"
"flat" = "out/f1""#,
        &["subdir/f1", "flat"],
        ""
        => vec![("out/f1".into(), vec!["flat".into(), "subdir".into()])]
        ; "literal_dir_vs_flat_file"
    )]
    #[test_case(
        r#""*.txt" = "text""#,
        &["a.txt", "b.txt"],
        ""
        => Vec::<(String, Vec<String>)>::new()
        ; "glob"
    )]
    #[test_case(
        r#""ghost" = "x""#,
        &[],
        ""
        => Vec::<(String, Vec<String>)>::new()
        ; "missing_source"
    )]
    #[test_case(
        r#""a" = "x"
"b" = "x"
"c" = "y"
"d" = "y""#,
        &["a", "b", "c", "d"],
        ""
        => vec![("x".into(), vec!["a".into(), "b".into()]), ("y".into(), vec!["c".into(), "d".into()])]
        ; "multiple_groups"
    )]
    #[test_case(
        r#""a" = "x""#,
        &["a"],
        "\"x\""
        => Vec::<(String, Vec<String>)>::new()
        ; "ignore_skips"
    )]
    fn test_resolve_collisions(
        portal: &str,
        files: &[&str],
        ignore: &str,
    ) -> Vec<(String, Vec<String>)> {
        let (_temp_dir, source_dir, target_dir) = setup_test(portal, ignore, "", false);
        let config = Config::read(&source_dir).unwrap();

        for f in files {
            let path = source_dir.join(f);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, "").unwrap();
        }

        let (_, mut collisions) =
            resolve_collisions(&config.portal, &config.ignore, &target_dir, &source_dir).unwrap();

        let mut result = Vec::new();
        for (k, mut v) in collisions.drain() {
            v.sort();
            let rel = Path::new(&k)
                .safe_strip_prefix(&target_dir)
                .display()
                .to_string();
            result.push((rel, v));
        }
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }

    #[test_case(
        &["x"],
        &[("x", "a")]
        => vec![("x".into(), vec!["a".to_string(), "x".to_string()])]
        ; "collision_with_marked"
    )]
    #[test_case(
        &["x"],
        &[("x", "a"), ("x", "b")]
        => vec![("x".into(), vec!["a".to_string(), "b".to_string(), "x".to_string(), "x".to_string()])]
        ; "multiple_new_same_target"
    )]
    #[test_case(
        &["x"],
        &[("y", "a")]
        => Vec::<(String, Vec<String>)>::new()
        ; "different_targets_no_collision"
    )]
    #[test_case(
        &[],
        &[("x", "a")]
        => Vec::<(String, Vec<String>)>::new()
        ; "empty_tree"
    )]
    // TODO add note when collide with dir
    #[test_case(
        &["dir/file"],
        &[("dir", "z")]
        => vec![("dir".into(), vec!["z".into()])]
        ; "file_collides_with_dir"
    )]
    fn test_check_new_entries(
        tree_paths: &[&str],
        entries: &[(&str, &str)],
    ) -> Vec<(String, Vec<String>)> {
        let (_temp_dir, source_dir, target_dir) = setup_test("", "", "", false);
        let mut tree = tree::Node::default();
        for p in tree_paths {
            tree.check_entry(&target_dir.join(p), p.to_string())
                .unwrap();
        }
        let entries: Vec<(PathBuf, PathBuf)> = entries
            .iter()
            .map(|(a, b)| (target_dir.join(a), source_dir.join(b)))
            .collect();
        let mut collisions = check_new_entries(&mut tree, &entries, &target_dir, &source_dir);
        let mut result = Vec::new();
        for (k, mut v) in collisions.drain() {
            v.sort();
            let rel = Path::new(&k)
                .safe_strip_prefix(&target_dir)
                .display()
                .to_string();
            result.push((rel, v));
        }
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }

    #[test]
    fn test_merge_collision_groups() {
        let existing = HashMap::from([("/a".to_string(), vec!["k1".to_string()])]);
        let new = HashMap::from([
            ("/a".to_string(), vec!["k1".to_string()]),
            ("/b".to_string(), vec!["k2".to_string()]),
        ]);
        let result = merge_collision_groups(existing, new);
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|(_, keys)| keys == &["k1"]));
        assert!(result.iter().any(|(_, keys)| keys == &["k2"]));
    }

    #[test_case(
        |s, _| { fs::write(s.join("a"), "").unwrap(); },
        r#""a" = """#,
        vec![("", "a")]
        => (Vec::<(String, String)>::new(), vec![(1, vec!["a".to_string()])])
        ; "single_literal_collision"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("a"), "").unwrap();
            fs::write(s.join("b"), "").unwrap();
        },
        r#""a" = "x"
"b" = "x""#,
        vec![]
        => (Vec::<(String, String)>::new(), vec![(1, vec!["a".to_string(), "b".to_string()])])
        ; "existing_portal_collision"
    )]
    #[test_case(
        |s, _| { fs::write(s.join("new"), "").unwrap(); },
        r#""a" = "x""#,
        vec![("", "new")]
        => (vec![("new".to_string(), "".to_string())], Vec::<(usize, Vec<String>)>::new())
        ; "missing_key"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("a"), "").unwrap();
            fs::write(s.join("new"), "").unwrap();
        },
        r#""a" = "x""#,
        vec![("x", "new")]
        => (
            vec![("new".to_string(), "x".to_string())],
            vec![(1, vec!["a".to_string(), "new".to_string()])],
        )
        ; "both_missing_and_collision"
    )]
    #[test_case(
        |s, _| {
            fs::create_dir(s.join("dir")).unwrap();
            fs::write(s.join("dir/a"), "").unwrap();
            fs::write(s.join("b"), "").unwrap();
            fs::write(s.join("c"), "").unwrap();
        },
        r#""dir" = ""
"b" = "a"
        "#,
        vec![("a", "c")]
        => (
            vec![("c".to_string(), "a".to_string())],
            vec![(1, vec!["b".to_string(), "c".to_string(), "dir".to_string()])],
        )
        ; "three_way_collision_at_same_target"
    )]
    fn test_analyze_portal(
        setup: impl FnOnce(&Path, &Path),
        portal: &str,
        entries: Vec<(&str, &str)>,
    ) -> (Vec<(String, String)>, Vec<(usize, Vec<String>)>) {
        let (_temp_dir, source_dir, target_dir) = setup_test(portal, "", "", false);
        let config = Config::read(&source_dir).unwrap();

        setup(&source_dir, &target_dir);

        let entries: Vec<(PathBuf, PathBuf)> = entries
            .iter()
            .map(|(a, b)| (target_dir.join(a), source_dir.join(b)))
            .collect();

        let analysis = analyze_portal(&config, &entries, &target_dir, &source_dir).unwrap();
        let missing: Vec<(String, String)> = analysis
            .missing
            .iter()
            .map(|(d, c)| (d.display().to_string(), c.display().to_string()))
            .collect();
        (missing, analysis.collisions)
    }

    #[test_case(
        r#""" = """#,
        |t, _| {
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), "file".into())
        },
        |_, _| {
            OPEN_EDITOR.with_borrow(|b| assert!(!*b));
        }; "move_editor_portal_match"
    )]
    #[test_case(
        r#""other" = """#,
        |t, _| {
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), "file".into())
        },
        |_, _| {
            OPEN_EDITOR.with_borrow(|b| assert!(*b));
        }; "move_editor_portal_mismatch"
    )]
    #[test_case(
        "",
        |t, _| {
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), "file".into())
        },
        |_, _| {
            OPEN_EDITOR.with_borrow(|b| assert!(*b));
        }; "move_editor_empty_portal"
    )]
    #[test_case(
        r#""" = ""
"other" = "file"
"#,
        |t, s| {
            fs::write(s.join("other"), "").unwrap();
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), "file".into())
        },
        |_, _| {
            OPEN_EDITOR.with_borrow(|b| assert!(*b));
        }; "move_editor_collision_only"
    )]
    #[test_case(
        r#""other" = "file""#,
        |t, s| {
            fs::write(s.join("other"), "").unwrap();
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), "file".into())
        },
        |_, _| {
            OPEN_EDITOR.with_borrow(|b| assert!(*b));
        }; "move_editor_collision_only_and_missing"
    )]
    fn test_add_open_editor(
        portal: &str,
        setup: impl FnOnce(&Path, &Path) -> (PathBuf, PathBuf),
        assertion: impl FnOnce(&Path, &Path),
    ) {
        let (temp_dir, source_dir, _) = setup_test(portal, "", "", false);
        let (f, d) = setup(temp_dir.path(), &source_dir);

        mock_add(&source_dir, &f, &d, FLAGS);

        assertion(&source_dir, temp_dir.path());
    }
}
