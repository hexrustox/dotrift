use std::{
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Command,
};

use miette::{Context, Result, miette};
use normalize_path::NormalizePath;

use crate::{
    cli::{AddFlags, GlobalFlags, OpenEditor},
    command::{
        to_absolute_path,
        util::{PathExt, copy_recursive, resolve_target, walk_files},
    },
    config::Config,
    copy_file_err,
    db::Db,
    global_config::GlobalConfig,
    output,
    path::{PKG_NAME, config_path},
    templater::{data::TemplateData, function::BuiltinFunctions},
};

use self::config::{expand_args, find_portal_cursor, prepare_config};
use self::portal::analyze_portal;

mod config;
mod portal;

pub use self::portal::PortalAnalysis;

pub fn run(
    global_flags: GlobalFlags,
    path: PathBuf,
    destination: Option<PathBuf>,
    flags: AddFlags,
    db_path: &Path,
) -> Result<()> {
    let source_dir = global_flags.source()?;
    let path = to_absolute_path(&path)?;

    let reimport = destination.is_none();

    let reimport_dir = path.path_is_dir() && reimport;

    let entries: Vec<(PathBuf, PathBuf)> = if reimport_dir {
        let db = Db::init(db_path)?;
        let mut entries = Vec::new();
        for entry in walk_files(&path) {
            let p = entry.path();
            match db.get_entry(p)? {
                Some(db_entry) => {
                    let dest = db_entry.source_path;
                    if !dest.starts_with(&source_dir) {
                        output::print_warn(format!(
                            "`{}` is outside source directory, skipping",
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
                    output::print_warn(format!("`{}` is not in database, skipping", p.display()));
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
                    return Err(miette!(
                        help = format!(
                            "deploy the path first with `{} apply`, or provide an explicit destination",
                            PKG_NAME
                        ),
                        "path `{}` not found in database",
                        path.display(),
                    ));
                }
            }
        };
        if !dest.starts_with(&source_dir) {
            return Err(miette!(
                "destination `{}` must be inside source directory `{}`",
                dest.display(),
                source_dir.display()
            ));
        }
        vec![(path, dest)]
    };

    if !reimport_dir {
        let dest = &entries[0].1;
        if dest.path_exists() && !flags.force {
            return Err(miette!(
                help = "use --force to overwrite the existing file, or remove it manually",
                "`{}` already exists",
                dest.display()
            ));
        }
    }

    let verbose = global_flags.verbose;
    for (src, dest) in &entries {
        if flags.force {
            remove_obstructions(dest, verbose)?;
        }
        if let Some(parent) = dest.parent() {
            crate::create_dir_err!(fs::create_dir_all(parent), parent)?;
        }
        if reimport || flags.copy {
            copy_recursive(src, dest)
        } else {
            fs::rename(src, dest)
                .map_err(|e| miette!(e))
                .wrap_err_with(|| {
                    format!("failed to move `{}` to `{}`", src.display(), dest.display())
                })
        }?;
        if verbose {
            output::print_added(src, dest);
        }
    }

    let db = Db::init(db_path)?;
    let data = TemplateData::read(&source_dir)?;
    let variables = data.resolve_variables(&db)?;
    let functions = BuiltinFunctions::new();
    let config = Config::read_templated(&source_dir, &variables, &functions)?;
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

    let config_override = global_flags.config()?;
    let temp_config = if !flags.no_modify && should_open {
        prepare_config(&source_dir, &analysis, &target_dir)?
    } else {
        None
    };

    if should_open {
        let config_path = config_path(&source_dir);
        if let Some(ref temp) = temp_config {
            let before = fs::metadata(temp).ok().and_then(|m| m.modified().ok());
            launch_editor(temp, config_override)?;
            let after = fs::metadata(temp).ok().and_then(|m| m.modified().ok());
            if before.is_some_and(|b| after.is_some_and(|a| a != b)) {
                copy_file_err!(fs::copy(temp, &config_path), temp, config_path)?;
            }
            let _ = fs::remove_file(temp);
        } else {
            launch_editor(&config_path, config_override)?;
        }
    }

    Ok(())
}

#[allow(unused)]
fn launch_editor(file_path: &Path, config_override: Option<PathBuf>) -> Result<()> {
    #[cfg(test)]
    return Ok(());

    let file = file_path.to_string_lossy().into_owned();
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
                .wrap_err("failed to expand editor command parameters")?;
            (config.command, args)
        }
        _ => match env::var_os("VISUAL")
            .or(env::var_os("EDITOR"))
            .filter(|s| !s.is_empty())
        {
            Some(cmd_os) => {
                let cmd_str = cmd_os.to_string_lossy();
                let mut parts = cmd_str.split_whitespace();
                let cmd = parts.next().expect("editor command is empty").to_string();
                let mut args: Vec<String> = parts.map(String::from).collect();
                args.push(file);
                (cmd, args)
            }
            None => {
                let config_path = crate::path::global_config_path()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| "$XDG_CONFIG_HOME/dotrift/config.toml".into());
                return Err(miette!(
                    help = format!(
                        "set $VISUAL or $EDITOR, or configure editor-command in {config_path}"
                    ),
                    "no editor found"
                ));
            }
        },
    };

    let status = Command::new(&cmd).args(&args).status();
    if let Err(ref e) = status {
        return if e.kind() == ErrorKind::NotFound {
            Err(miette!("editor command not found: `{cmd}`"))
        } else {
            status
                .map(|_| ())
                .map_err(|e| miette!(e))
                .wrap_err_with(|| {
                    format!(
                        "failed to launch editor: `{}`",
                        [vec![cmd], args].concat().join(" ")
                    )
                })
        };
    }

    Ok(())
}

fn remove_obstructions(path: &Path, verbose: bool) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.is_dir() {
            crate::remove_dir_err!(fs::remove_dir_all(path), path)?;
            if verbose {
                output::print_removed(path);
            }
        } else {
            crate::remove_file_err!(fs::remove_file(path), path)?;
            if verbose {
                output::print_removed(path);
            }
        }
    }
    for dir in path.ancestors().skip(1) {
        let Ok(meta) = fs::symlink_metadata(dir) else {
            continue;
        };
        if meta.is_dir() {
            break;
        }
        crate::remove_file_err!(fs::remove_file(dir), dir)?;
        if verbose {
            output::print_removed(dir);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, fs, os::unix::fs as unix_fs, path::Path};

    use crate::{cli::AddFlags, command::util::tests::setup_test, config::DeployType, db::DbEntry};

    use super::*;
    use test_case::test_case;

    pub const FLAGS: AddFlags = AddFlags {
        copy: false,
        force: false,
        editor: None,
        no_modify: false,
    };

    pub fn mock_add(source_dir: &Path, path: &Path, destination: &Path, flags: AddFlags) {
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

    pub fn mock_add_reimport(source_dir: &Path, path: &Path, flags: AddFlags, db_path: &Path) {
        run(
            GlobalFlags::new(Some(source_dir.to_path_buf()), None, None),
            path.to_path_buf(),
            None,
            flags,
            db_path,
        )
        .unwrap();
    }

    thread_local! {
        pub static OPEN_EDITOR: RefCell<bool> = const { RefCell::new(false) };
    }

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
    #[test_case(
        |t, s| {
            fs::write(t.join("file"), "data").unwrap();
            (t.join("file"), s.join("a/b/c/file"))
        },
        |s, _| {
            assert!(s.join("a/b/c/file").exists());
        }; "move_to_nested_dest"
    )]
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
                mtime: None,
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
                mtime: None,
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
                    mtime: None,
                },
                DbEntry {
                    target_path: t.join("dir/sub/file2"),
                    deploy_type: DeployType::Copy,
                    source_path: s.join("dest/sub/file2"),
                    hash: None,
                    symlink_target: None,
                    mtime: None,
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
                    mtime: None,
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
        ; "obstruction_file_at_dest"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("x")).unwrap();
            t.join("x")
        },
        |t| {
            assert!(!t.join("x").exists());
        }
        ; "obstruction_empty_dir_at_dest"
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
        ; "obstruction_nonempty_dir_at_dest"
    )]
    #[test_case(
        |t| {
            unix_fs::symlink(Path::new("/nonexistent"), t.join("link")).unwrap();
            t.join("link")
        },
        |t| {
            assert!(!t.join("link").is_symlink());
        }
        ; "obstruction_dangling_symlink_at_dest"
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
        ; "obstruction_valid_symlink_at_dest"
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
        ; "obstruction_symlink_to_dir_at_dest"
    )]
    #[test_case(
        |t| {
            t.join("ghost")
        },
        |_| {}
        ; "obstruction_nonexistent_dest"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("a"), "block").unwrap();
            t.join("a/b/c")
        },
        |t| {
            assert!(!t.join("a").exists());
        }
        ; "obstruction_file_blocking_parent"
    )]
    #[test_case(
        |t| {
            unix_fs::symlink(Path::new("/nonexistent"), t.join("a")).unwrap();
            t.join("a/b/c")
        },
        |t| {
            assert!(!t.join("a").is_symlink());
        }
        ; "obstruction_dangling_symlink_blocking_parent"
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
        ; "obstruction_valid_symlink_blocking_parent"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("dir")).unwrap();
            t.join("dir/target")
        },
        |t| {
            assert!(t.join("dir").is_dir());
        }
        ; "obstruction_none_with_parent"
    )]
    #[test_case(
        |t| {
            t.join("a/b/c")
        },
        |_| {}
        ; "obstruction_none_without_parent"
    )]
    fn test_remove_obstructions(setup: impl FnOnce(&Path) -> PathBuf, assert: impl FnOnce(&Path)) {
        let t = tempfile::tempdir().unwrap();
        let target = setup(t.path());
        remove_obstructions(&target, false).unwrap();
        assert(t.path());
    }

    #[test_case(
        r#""" = """#,
        |t, _| {
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), "file".into())
        },
        |_, _| {
            OPEN_EDITOR.with_borrow(|b| assert!(!b));
        }        ; "editor_closed_when_destination_matches_portal"
    )]
    #[test_case(
        r#""other" = """#,
        |t, _| {
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), "file".into())
        },
        |_, _| {
            OPEN_EDITOR.with_borrow(|b| assert!(b));
        }        ; "editor_opens_when_portal_mismatch"
    )]
    #[test_case(
        "",
        |t, _| {
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), "file".into())
        },
        |_, _| {
            OPEN_EDITOR.with_borrow(|b| assert!(b));
        }        ; "editor_opens_when_portal_empty"
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
            OPEN_EDITOR.with_borrow(|b| assert!(b));
        }        ; "editor_opens_when_collision_exists"
    )]
    #[test_case(
        r#""other" = "file""#,
        |t, s| {
            fs::write(s.join("other"), "").unwrap();
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), "file".into())
        },
        |_, _| {
            OPEN_EDITOR.with_borrow(|b| assert!(b));
        }        ; "editor_opens_when_collision_and_missing"
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
