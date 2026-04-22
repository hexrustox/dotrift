use std::{
    env,
    fs::{self, remove_dir_all, remove_file},
    path::{Path, PathBuf},
    process::Command,
};

use color_eyre::eyre::{Context, Result, eyre};
use glob::Pattern;
use normalize_path::NormalizePath;

use crate::{
    cli::{AddFlags, OpenEditor},
    command::util::{GLOB_OPTION, PathLiteral, copy_recursive, is_glob},
    config::Config,
    error::{GlobError, IoError},
    global_config::GlobalConfig,
    path::{config_path, global_config_path},
};

pub fn run(
    source_dir: PathBuf,
    config_override: Option<PathBuf>,
    file: PathBuf,
    destination: PathBuf,
    flags: AddFlags,
) -> Result<()> {
    let destination = if destination.is_absolute() {
        destination
    } else {
        source_dir.join(destination).normalize()
    };
    let Ok(dest_rel) = destination.strip_prefix(&source_dir) else {
        return Err(eyre!("Destination must be inside source directory"));
    };
    if destination.literal_exists() && !flags.force {
        return Err(eyre!("`{}` already exists", destination.display()));
    }

    #[allow(unused_mut)]
    let mut open_editor = if let Some(open_editor) = flags.editor {
        matches!(open_editor, OpenEditor::Always)
    } else {
        match Config::read(&source_dir) {
            Ok(config) => {
                let mut open_editor = true;
                for (pattern, _) in config.portal {
                    if is_glob(&pattern) && {
                        let glob = Pattern::new(&pattern).glob_error()?;
                        glob.matches_path_with(dest_rel, GLOB_OPTION)
                    } || *dest_rel == *pattern
                        || {
                            let mut bool = false;
                            let mut current = dest_rel.parent();
                            while let Some(parent) = current {
                                if *parent == *pattern {
                                    bool = true;
                                    break;
                                }
                                current = parent.parent();
                            }
                            bool
                        }
                    {
                        open_editor = false;
                        break;
                    }
                }
                open_editor
            }
            Err(_) => true,
        }
    };

    #[cfg(test)]
    {
        tests::OPEN_EDITOR.set(open_editor);
        open_editor = false;
    }

    if open_editor {
        let specific_config = config_override.is_some();
        let path = config_override.unwrap_or(global_config_path());
        let (cmd, mut args) = match GlobalConfig::read(&path) {
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
        args.push(config_path(&source_dir).to_string_lossy().to_string());
        Command::new(&cmd).args(&args).status().wrap_err_with(|| {
            format!(
                "Failed to spawn process `{}`",
                [vec![cmd], args].concat().join(" ")
            )
        })?;
    }

    if flags.force {
        remove_obstructions(&destination)?;
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).create_dir_error(parent)?;
    }

    if flags.copy {
        copy_recursive(&file, &destination)?;
    } else {
        fs::rename(&file, &destination).wrap_err_with(|| {
            format!(
                "Failed to move `{}` to `{}`",
                file.display(),
                destination.display()
            )
        })?;
    }

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
    let mut current = path.parent();
    while let Some(dir) = current {
        if let Ok(meta) = fs::symlink_metadata(dir) {
            if meta.is_dir() {
                break;
            } else {
                remove_file(dir).remove_file_error(dir)?;
            }
        }
        current = dir.parent();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, os::unix::fs as unix_fs, path::Path};

    use crate::command::util::tests::setup_test;

    use super::*;
    use tempfile::TempDir;
    use test_case::test_case;

    // --- simple file moves (destination path variations) ---
    #[test_case(|t, _| {
        fs::write(t.join("file"), "").unwrap();
        (t.join("file"), "file".into())
    }, |s, _| {
        assert!(s.join("file").exists());
    }; "move_relative_path")]
    #[test_case(|t, s| {
        fs::write(t.join("file"), "").unwrap();
        (t.join("file"), s.join("file"))
    }, |s, t| {
        assert!(!t.join("file").exists());
        assert!(s.join("file").exists());
    }; "move_absolute_path")]
    #[test_case(|t, _| {
        fs::write(t.join("file"), "").unwrap();
        (t.join("file"), "./dir/../file".into())
    }, |s, _| {
        assert!(s.join("file").exists());
    }; "move_normalized_path")]
    // --- source type variations ---
    #[test_case(|t, s| {
        fs::write(t.join("real"), "data").unwrap();
        unix_fs::symlink(t.join("real"), t.join("link")).unwrap();
        (t.join("link"), s.join("link"))
    }, |s, t| {
        assert!(s.join("link").is_symlink());
        assert_eq!(
            fs::read_link(s.join("link")).unwrap(),
            t.join("real")
        );
    }; "move_symlink")]
    #[test_case(|t, s| {
        fs::create_dir_all(t.join("dir/sub")).unwrap();
        fs::write(t.join("dir/sub/file"), "data").unwrap();
        (t.join("dir"), s.join("dir"))
    }, |s, _| {
        assert!(s.join("dir").is_dir());
        assert!(s.join("dir/sub/file").exists());
    }; "move_directory")]
    // --- destination path depth ---
    #[test_case(|t, s| {
        fs::write(t.join("file"), "data").unwrap();
        (t.join("file"), s.join("a/b/c/file"))
    }, |s, _| {
        assert!(s.join("a/b/c/file").exists());
    }; "move_to_nested_dest")]
    // --- failure cases ---
    #[test_case(|_, s| {
        (s.join("nonexistent"), s.join("dest"))
    }, |_, _| {} => panics "move"; "fail_missing_source")]
    #[test_case(|t, s| {
        fs::write(t.join("file"), "").unwrap();
        fs::write(s.join("file"), "").unwrap();
        (t.join("file"), s.join("file"))
    }, |_, _| {} => panics "already exists"; "fail_dest_exists")]
    #[test_case(|t, _| {
        (t.join("file"), "../file".into())
    }, |_, _| {} => panics "inside"; "fail_escape_source")]
    fn test_add_move(
        setup: impl FnOnce(&Path, &Path) -> (PathBuf, PathBuf),
        assertion: impl FnOnce(&Path, &Path),
    ) {
        let temp_dir = TempDir::new().unwrap();
        let source_dir = temp_dir.path().join("source");
        fs::create_dir_all(&source_dir).unwrap();
        let (f, d) = setup(temp_dir.path(), &source_dir);

        run(
            source_dir.clone(),
            None,
            f,
            d,
            AddFlags {
                copy: false,
                force: false,
                editor: None,
            },
        )
        .unwrap();

        assertion(&source_dir, temp_dir.path());
    }

    #[test_case(
        |tmp| {
            fs::write(tmp.join("x"), "data").unwrap();
            tmp.join("x")
        },
        |tmp| {
            assert!(!tmp.join("x").exists());
        }
        ; "file_at_dest"
    )]
    #[test_case(
        |tmp| {
            fs::create_dir(tmp.join("x")).unwrap();
            tmp.join("x")
        },
        |tmp| {
            assert!(!tmp.join("x").exists());
        }
        ; "empty_dir_at_dest"
    )]
    #[test_case(
        |tmp| {
            let x = tmp.join("x");
            fs::create_dir_all(x.join("sub/nested")).unwrap();
            fs::write(x.join("sub/nested/f"), "data").unwrap();
            x
        },
        |tmp| {
            assert!(!tmp.join("x").exists());
        }
        ; "nonempty_dir_at_dest"
    )]
    #[test_case(
        |tmp| {
            unix_fs::symlink(Path::new("/nonexistent"), tmp.join("link")).unwrap();
            tmp.join("link")
        },
        |tmp| {
            assert!(!tmp.join("link").is_symlink());
        }
        ; "dangling_symlink_at_dest"
    )]
    #[test_case(
        |tmp| {
            let real = tmp.join("real");
            fs::write(&real, "data").unwrap();
            unix_fs::symlink(&real, tmp.join("link")).unwrap();
            tmp.join("link")
        },
        |tmp| {
            assert!(!tmp.join("link").is_symlink());
            assert!(tmp.join("real").exists());
        }
        ; "valid_symlink_file_at_dest"
    )]
    #[test_case(
        |tmp| {
            let real = tmp.join("dir");
            fs::create_dir(&real).unwrap();
            fs::write(real.join("f"), "data").unwrap();
            unix_fs::symlink(&real, tmp.join("link")).unwrap();
            tmp.join("link")
        },
        |tmp| {
            assert!(!tmp.join("link").is_symlink());
            assert!(tmp.join("dir/f").exists());
        }
        ; "symlink_to_dir_at_dest"
    )]
    #[test_case(
        |tmp| {
            tmp.join("ghost")
        },
        |_| {}
        ; "nonexistent_dest"
    )]
    #[test_case(
        |tmp| {
            fs::write(tmp.join("a"), "block").unwrap();
            tmp.join("a/b/c")
        },
        |tmp| {
            assert!(!tmp.join("a").exists());
        }
        ; "file_blocking_parent"
    )]
    #[test_case(
        |tmp| {
            unix_fs::symlink(Path::new("/nonexistent"), tmp.join("a")).unwrap();
            tmp.join("a/b/c")
        },
        |tmp| {
            assert!(!tmp.join("a").is_symlink());
        }
        ; "dangling_symlink_blocking_parent"
    )]
    #[test_case(
        |tmp| {
            let real = tmp.join("real");
            fs::write(&real, "data").unwrap();
            unix_fs::symlink(&real, tmp.join("a")).unwrap();
            tmp.join("a/b/c")
        },
        |tmp| {
            assert!(!tmp.join("a").is_symlink());
            assert!(tmp.join("real").exists());
        }
        ; "valid_symlink_blocking_parent"
    )]
    #[test_case(
        |tmp| {
            fs::create_dir(tmp.join("dir")).unwrap();
            tmp.join("dir/target")
        },
        |tmp| {
            assert!(tmp.join("dir").is_dir());
        }
        ; "no_obstruction_existing_parent"
    )]
    #[test_case(
        |tmp| {
            tmp.join("a/b/c")
        },
        |_| {}
        ; "no_obstruction_no_parent"
    )]
    #[test_case(
        |tmp| {
            fs::write(tmp.join("x"), "dest_content").unwrap();
            tmp.join("x")
        },
        |tmp| {
            assert!(!tmp.join("x").exists());
        }
        ; "file_at_dest_clean_parent"
    )]
    fn test_remove_obstructions(setup: impl FnOnce(&Path) -> PathBuf, assert: impl FnOnce(&Path)) {
        let tmp = tempfile::tempdir().unwrap();
        let target = setup(tmp.path());
        remove_obstructions(&target).unwrap();
        assert(tmp.path());
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
    fn test_open_editor(portal: &str, dest: impl Into<PathBuf>) -> bool {
        let (temp_dir, source_dir, _) = setup_test(portal, "", "", false);

        let path = temp_dir.path().join("file");
        fs::write(&path, "").unwrap();
        run(
            source_dir.clone(),
            None,
            path,
            source_dir.join(dest.into()),
            AddFlags {
                copy: false,
                force: false,
                editor: None,
            },
        )
        .unwrap();

        OPEN_EDITOR.with_borrow(|b| *b)
    }
}
