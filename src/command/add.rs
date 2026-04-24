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
    command::util::{GLOB_OPTION, PathLiteral, SafeStripPrefix, copy_recursive, is_glob},
    config::Config,
    db::Db,
    error::IoError,
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

    let reimport = destination.is_none();

    if path.is_literal_dir() && reimport {
        return reimport_directory(&source_dir, &config_override, &path, flags, db_path);
    }

    let destination = if let Some(dest) = destination {
        if dest.is_absolute() {
            dest
        } else {
            source_dir.join(dest).normalize()
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

    let Ok(dest_rel) = destination.strip_prefix(&source_dir) else {
        return Err(eyre!("Destination must be inside source directory"));
    };
    if destination.literal_exists() && !flags.force {
        return Err(eyre!("`{}` already exists", destination.display()));
    }

    let should_open = needs_editor(&source_dir, [dest_rel], flags.editor)?;

    #[cfg(test)]
    {
        tests::OPEN_EDITOR.set(should_open);
    }

    if should_open {
        launch_editor(&source_dir, &config_override)?;
    }

    if flags.force {
        remove_obstructions(&destination)?;
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).create_dir_error(parent)?;
    }

    if reimport || flags.copy {
        copy_recursive(&path, &destination)?;
    } else {
        fs::rename(&path, &destination).wrap_err_with(|| {
            format!(
                "Failed to move `{}` to `{}`",
                path.display(),
                destination.display()
            )
        })?;
    }

    Ok(())
}

fn reimport_directory(
    source_dir: &Path,
    config_override: &Option<PathBuf>,
    dir: &Path,
    flags: AddFlags,
    db_path: &Path,
) -> Result<()> {
    let db = Db::init(db_path)?;

    let mut entries = Vec::new();
    for entry in WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|e| !e.file_type().is_dir())
    {
        let path = entry.path();
        match db.get_entry(path)? {
            Some(db_entry) => {
                let dest = db_entry.source_path;
                if !dest.starts_with(source_dir) {
                    eprintln!(
                        "Warning: `{}` destination outside source dir, skipping",
                        path.display()
                    );
                    continue;
                }
                if dest.literal_exists() && !flags.force {
                    eprintln!("Warning: `{}` already exists, skipping", dest.display());
                    continue;
                }
                entries.push((path.to_path_buf(), dest));
            }
            None => {
                eprintln!("Warning: `{}` not in database, skipping", path.display());
            }
        }
    }

    let should_open = needs_editor(
        source_dir,
        entries.iter().map(|(_, p)| p.safe_strip_prefix(source_dir)),
        flags.editor,
    )?;

    #[cfg(test)]
    {
        tests::OPEN_EDITOR.set(should_open);
    }

    if should_open {
        launch_editor(source_dir, config_override)?;
    }

    for (entry_path, dest) in entries {
        if flags.force {
            remove_obstructions(&dest)?;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).create_dir_error(parent)?;
        }
        copy_recursive(&entry_path, &dest)?;
    }

    Ok(())
}

fn needs_editor<'a>(
    source_dir: &Path,
    destinations: impl IntoIterator<Item = &'a Path>,
    open_editor: Option<OpenEditor>,
) -> Result<bool> {
    match open_editor {
        Some(OpenEditor::Always) => Ok(true),
        Some(OpenEditor::Never) => Ok(false),
        None => {
            let config = Config::read(source_dir)?;
            Ok(destinations
                .into_iter()
                .any(|p| !portal_matches(p, &config.portal)))
        }
    }
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

#[cfg(test)]
fn launch_editor(_: &Path, _: &Option<PathBuf>) -> Result<()> {
    Ok(())
}

#[cfg(not(test))]
fn launch_editor(source_dir: &Path, config_override: &Option<PathBuf>) -> Result<()> {
    use std::{env, process::Command};

    use crate::{
        global_config::GlobalConfig,
        path::{config_path, global_config_path},
    };

    let specific_config = config_override.is_some();
    let default_path = global_config_path();
    let path = config_override
        .as_ref()
        .map(|p| p.as_path())
        .unwrap_or(&default_path);
    let (cmd, mut args) = match GlobalConfig::read(path) {
        Ok(GlobalConfig {
            editor_command: Some(config),
            ..
        }) => (config.command, config.args),
        Err(err) if specific_config => {
            return Err(err);
        }
        _ => match env::var_os("VISUAL").or(env::var_os("EDITOR")) {
            Some(cmd) => (cmd.to_string_lossy().to_string(), Vec::new()),
            None => return Err(eyre!("Failed to open editor")),
        },
    };
    args.push(config_path(source_dir).to_string_lossy().to_string());
    Command::new(&cmd).args(&args).status().wrap_err_with(|| {
        format!(
            "Failed to spawn process `{}`",
            [vec![cmd], args].concat().join(" ")
        )
    })?;
    Ok(())
}

fn remove_obstructions(path: &Path) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.is_dir() {
            remove_dir_all(path).remove_dir_error(path)?;
        } else {
            remove_file(path).remove_file_error(path)?;
        }
    }
    for dir in path.ancestors().skip(1) {
        let Ok(meta) = fs::symlink_metadata(dir) else {
            continue;
        };
        if meta.is_dir() {
            break;
        }
        remove_file(dir).remove_file_error(dir)?;
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
    fn test_open_editor(portal: &str, dest: &str) -> bool {
        let (_temp_dir, source_dir, _) = setup_test(portal, "", "", false);
        let config = Config::read(&source_dir).unwrap();

        !portal_matches(Path::new(dest), &config.portal)
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
    fn test_add_move_editor(
        portal: &str,
        setup: impl FnOnce(&Path, &Path) -> (PathBuf, PathBuf),
        assertion: impl FnOnce(&Path, &Path),
    ) {
        let (temp_dir, source_dir, _) = setup_test(portal, "", "", false);
        let (f, d) = setup(temp_dir.path(), &source_dir);

        mock_add(&source_dir, &f, &d, FLAGS);

        assertion(&source_dir, temp_dir.path());
    }

    #[test_case(
        r#""" = """#,
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
        |_| {
            OPEN_EDITOR.with_borrow(|b| assert!(!*b));
        }; "reimport_editor_portal_match"
    )]
    #[test_case(
        r#""other" = """#,
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
        |_| {
            OPEN_EDITOR.with_borrow(|b| assert!(*b));
        }; "reimport_editor_portal_mismatch"
    )]
    #[test_case(
        "",
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
        |_| {
            OPEN_EDITOR.with_borrow(|b| assert!(*b));
        }; "reimport_editor_empty_portal"
    )]
    #[test_case(
        r#""" = """#,
        |t, s| {
            fs::create_dir_all(t.join("dir")).unwrap();
            fs::write(t.join("dir/file1"), "a").unwrap();
            (t.join("dir"), vec![
                DbEntry {
                    target_path: t.join("dir/file1"),
                    deploy_type: DeployType::Copy,
                    source_path: s.join("file1"),
                    hash: None,
                    symlink_target: None,
                },
            ])
        },
        |_| {
            OPEN_EDITOR.with_borrow(|b| assert!(!*b));
        }; "reimport_editor_dir_portal_match"
    )]
    fn test_add_reimport_editor(
        portal: &str,
        setup: impl FnOnce(&Path, &Path) -> (PathBuf, Vec<DbEntry>),
        assertion: impl FnOnce(&Path),
    ) {
        let (temp_dir, source_dir, _) = setup_test(portal, "", "", false);
        let db_path = &temp_dir.path().join("db");
        let (p, es) = setup(temp_dir.path(), &source_dir);

        let db = Db::init(db_path).unwrap();
        for e in es {
            db.insert_or_update(&e).unwrap();
        }
        mock_add_reimport(&source_dir, &p, FLAGS, db_path);
        assertion(&source_dir);
    }
}
