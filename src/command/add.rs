use std::{
    env,
    fs::{self, remove_dir, remove_dir_all, remove_file},
    path::PathBuf,
    process::Command,
};

use color_eyre::eyre::{Context, eyre};
use glob::Pattern;
use normalize_path::NormalizePath;

use crate::{
    cli::{AddFlags, OpenEditor},
    command::util::{GLOB_OPTION, PathKind, copy_recursive, is_glob},
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
) -> color_eyre::Result<()> {
    let destination = if destination.is_absolute() {
        destination
    } else {
        source_dir.join(destination).normalize()
    };
    let Ok(dest_rel) = destination.strip_prefix(&source_dir) else {
        return Err(eyre!("Destination must be inside source directory"));
    };
    if destination.exists() && !flags.force {
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

    if file.is_dir_kind() {
        if flags.force && destination.exists() {
            if destination.is_dir_kind() {
                remove_dir_all(&destination).remove_dir_error(&destination)?;
            } else {
                remove_file(&destination).remove_file_error(&destination)?;
            }
        }
    } else if let Some(parent) = destination.parent() {
        if flags.force {
            if destination.exists() && destination.is_dir_kind() {
                remove_dir(&destination).remove_dir_error(&destination)?;
            }
            let mut current = Some(parent);
            while let Some(path) = current {
                if path.exists() && !path.is_dir_kind() {
                    fs::remove_file(path).remove_file_error(path)?;
                    break;
                }
                current = path.parent();
            }
        }
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

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, os::unix::fs as unix_fs, path::Path};

    use crate::command::util::tests::setup_test;

    use super::*;
    use tempfile::TempDir;
    use test_case::test_case;

    #[test_case(|t, s| {
        fs::write(t.join("file"), "").unwrap();
        (t.join("file"), s.join("file"))
    }, |s| {
        assert!(s.join("file").exists());
    }; "move_absolute_path")]
    #[test_case(|t, _| {
        fs::write(t.join("file"), "").unwrap();
        (t.join("file"), "file".into())
    }, |s| {
        assert!(s.join("file").exists());
    }; "move_relative_path")]
    #[test_case(|t, _| {
        fs::write(t.join("file"), "").unwrap();
        (t.join("file"), "./dir/../file".into())
    }, |s| {
        assert!(s.join("file").exists());
    }; "move_normalized_path")]
    #[test_case(|t, s| {
        (t.join("file"), s.join("file"))
    }, |_| {} => panics "move"; "fail_missing_source")]
    #[test_case(|t, s| {
        fs::write(t.join("file"), "").unwrap();
        fs::write(s.join("file"), "").unwrap();
        (t.join("file"), s.join("file"))
    }, |_| {} => panics "already exists"; "fail_dest_exists")]
    #[test_case(|t, _| {
        (t.join("file"), "../file".into())
    }, |_| {} => panics "inside"; "fail_escape_source")]
    fn test_add(
        setup: impl FnOnce(&Path, &Path) -> (PathBuf, PathBuf),
        assertion: impl FnOnce(&Path),
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

        assertion(&source_dir);
    }

    #[test_case(|t, s| {
        fs::write(t.join("file"), "").unwrap();
        (t.join("file"), s.join("file"))
    }, |t, s| {
        assert!(t.join("file").exists());
        assert!(s.join("file").exists());
    }; "1")]
    #[test_case(|t, s| {
        unix_fs::symlink(Path::new("/a"), t.join("file")).unwrap();
        (t.join("file"), s.join("file"))
    }, |_, s| {
        assert_eq!(fs::read_link(s.join("file")).unwrap(), Path::new("/a"));
    }; "2")]
    #[test_case(|t, s| {
        fs::create_dir_all(t.join("dir")).unwrap();
        fs::write(t.join("dir/file"), "").unwrap();
        (t.join("dir"), s.join("dir"))
    }, |_, s| {
        assert!(s.join("dir").exists());
        assert!(s.join("dir/file").exists());
    }; "3")]
    fn test_add_copy(
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
                copy: true,
                force: false,
                editor: None,
            },
        )
        .unwrap();

        assertion(temp_dir.path(), &source_dir);
    }

    #[test_case(|t, s| {
        fs::write(t.join("file"), "a").unwrap();
        fs::write(s.join("file"), "b").unwrap();
        (t.join("file"), s.join("file"))
    }, |s| {
        assert_eq!(fs::read_to_string(s.join("file")).unwrap(), "a");
    }; "overwrite_existing")]
    #[test_case(|t, s| {
        fs::write(t.join("file"), "").unwrap();
        fs::create_dir_all(s.join("file")).unwrap();
        (t.join("file"), s.join("file"))
    }, |s| {
        assert!(s.join("file").is_file());
    }; "overwrite_existing_2")]
    #[test_case(|t, s| {
        fs::create_dir_all(t.join("dir")).unwrap();
        fs::write(t.join("dir/file"), "").unwrap();
        fs::write(s.join("dir"), "").unwrap();
        (t.join("dir"), s.join("dir"))
    }, |s| {
        assert!(s.join("dir/file").exists());
    }; "overwrite_existing_3")]
    #[test_case(|t, s| {
        fs::write(t.join("file"), "").unwrap();
        fs::write(s.join("dir"), "").unwrap();
        (t.join("file"), s.join("dir/subdir/file"))
    }, |s| {
        assert!(s.join("dir/subdir/file").exists());
    }; "clear_path_obstruction")]
    fn test_add_force(
        setup: impl FnOnce(&Path, &Path) -> (PathBuf, PathBuf),
        assertion: impl FnOnce(&Path),
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
                force: true,
                editor: None,
            },
        )
        .unwrap();

        assertion(&source_dir);
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
