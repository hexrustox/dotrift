use std::{
    fmt::Display,
    fs,
    io::Write,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    process::{Command, Stdio},
};

#[cfg(any(test, feature = "testing"))]
use std::cell::RefCell;

use miette::{Result, WrapErr, miette};
use similar::TextDiff;
use strum::EnumIter;
use tui::prompt::{PromptError, PromptOption};

use crate::ExitStatus;
use crate::config::{self, DeployType};
use crate::hash;
use crate::managed;
use crate::state::{Kind, StateDatabase, StateLock, StateRecord};
use crate::template;

/// Reconciles the desired deployment with the target directory.
pub fn run(source: &Path, target_override: Option<std::path::PathBuf>) -> Result<ExitStatus> {
    let _lock = StateLock::acquire()?;
    let deployment = config::read(source, target_override)?;
    let target = &deployment.target_directory;

    if fs::symlink_metadata(target)
        .map(|metadata| !metadata.file_type().is_dir())
        .unwrap_or(false)
    {
        return Err(miette!(
            "target directory `{}` is not a directory",
            target.display()
        ));
    }
    if deployment.entries.is_empty() {
        return Ok(ExitStatus::Success);
    }
    if fs::symlink_metadata(target).is_err() {
        fs::create_dir_all(target)
            .map_err(|error| miette!(error).wrap_err("cannot create target directory"))?;
    }

    let database = StateDatabase::open()?;
    let mut entries = deployment.entries;
    entries.sort_by(|left, right| left.target_path.cmp(&right.target_path));
    let mut replace_all = false;
    let mut skipped = 0;
    let mut deployed = 0;
    let mut replaced = 0;
    for entry in entries {
        match deploy_entry(
            &database,
            target,
            &entry,
            &deployment.variable_context,
            &mut replace_all,
        )? {
            EntryResult::Deployed => deployed += 1,
            EntryResult::Replaced => replaced += 1,
            EntryResult::Skipped => skipped += 1,
            EntryResult::Cancelled => return Ok(ExitStatus::Cancelled),
        }
    }
    println!("deployed {deployed}, replaced {replaced}, skipped {skipped}");
    if skipped > 0 {
        return Ok(ExitStatus::Skipped);
    }
    Ok(ExitStatus::Success)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryResult {
    Deployed,
    Replaced,
    Skipped,
    Cancelled,
}

fn deploy_entry(
    database: &StateDatabase,
    target_root: &Path,
    entry: &config::DeploymentEntry,
    context: &std::collections::HashMap<String, templater::value::Value>,
    replace_all: &mut bool,
) -> Result<EntryResult> {
    if !fs::metadata(&entry.source_path)
        .map_err(|error| miette!(error))?
        .is_file()
    {
        return Err(miette!(
            "source path `{}` is no longer a regular file",
            entry.source_path.display()
        ));
    }

    let obstruction = parent_obstruction(target_root, &entry.target_path)?;
    let existed = if obstruction.is_some() {
        false
    } else {
        match fs::symlink_metadata(&entry.target_path) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(miette!(error).wrap_err(format!(
                    "cannot inspect target `{}`",
                    entry.target_path.display()
                )));
            }
        }
    };
    let mut replaced = false;
    if let Some(obstruction) = obstruction {
        if !*replace_all {
            loop {
                match prompt_for_obstruction(entry, &obstruction) {
                    Ok(ObstructionChoice::Skip) => return Ok(EntryResult::Skipped),
                    Ok(ObstructionChoice::ViewDiff) => show_diff(entry, &obstruction, context)?,
                    Ok(ObstructionChoice::Replace) => {
                        remove_path(database, &obstruction)?;
                        replaced = true;
                        break;
                    }
                    Ok(ObstructionChoice::ReplaceAll) => {
                        *replace_all = true;
                        remove_path(database, &obstruction)?;
                        replaced = true;
                        break;
                    }
                    Err(PromptError::Cancelled) => return Ok(EntryResult::Cancelled),
                    Err(error) => {
                        return Err(miette!(error).wrap_err("cannot display obstruction prompt"));
                    }
                }
            }
        } else {
            remove_path(database, &obstruction)?;
            replaced = true;
        }
    } else if existed {
        let old_record = database.record(&entry.target_path)?;
        let managed = old_record
            .as_ref()
            .map(managed::is_managed)
            .transpose()?
            .unwrap_or(false);
        if managed {
            remove_path(database, &entry.target_path)?;
            replaced = true;
        } else if !*replace_all {
            loop {
                match prompt_for_obstruction(entry, &entry.target_path) {
                    Ok(ObstructionChoice::Skip) => return Ok(EntryResult::Skipped),
                    Ok(ObstructionChoice::ViewDiff) => {
                        show_diff(entry, &entry.target_path, context)?
                    }
                    Ok(ObstructionChoice::Replace) => {
                        remove_path(database, &entry.target_path)?;
                        replaced = true;
                        break;
                    }
                    Ok(ObstructionChoice::ReplaceAll) => {
                        *replace_all = true;
                        remove_path(database, &entry.target_path)?;
                        replaced = true;
                        break;
                    }
                    Err(PromptError::Cancelled) => return Ok(EntryResult::Cancelled),
                    Err(error) => {
                        return Err(miette!(error).wrap_err("cannot display obstruction prompt"));
                    }
                }
            }
        } else {
            remove_path(database, &entry.target_path)?;
            replaced = true;
        }
    }
    let parent = entry
        .target_path
        .parent()
        .ok_or_else(|| miette!("target path has no parent"))?;
    fs::create_dir_all(parent)
        .map_err(|error| miette!(error))
        .wrap_err("cannot create target parent directories")?;

    let record = match entry.deploy_type {
        DeployType::Symlink => {
            symlink(&entry.source_path, &entry.target_path)
                .map_err(|error| miette!(error))
                .wrap_err("cannot create target symlink")?;
            StateRecord {
                target_path: entry.target_path.clone(),
                source_path: entry.source_path.clone(),
                kind: Kind::Symlink,
                link_target: Some(entry.source_path.clone()),
                content_hash: None,
            }
        }
        DeployType::Copy | DeployType::Template => {
            let bytes = if entry.deploy_type == DeployType::Template {
                template::render_template(&entry.source_path, context)?
            } else {
                fs::read(&entry.source_path)
                    .map_err(|error| miette!(error))
                    .wrap_err("cannot read copy source")?
            };
            fs::write(&entry.target_path, &bytes)
                .map_err(|error| miette!(error))
                .wrap_err("cannot write target file")?;
            StateRecord {
                target_path: entry.target_path.clone(),
                source_path: entry.source_path.clone(),
                kind: Kind::File,
                link_target: None,
                content_hash: Some(hash::hash_bytes(&bytes)),
            }
        }
    };
    database.put(&record)?;
    if let Some(mode) = entry.mode {
        fs::set_permissions(&entry.target_path, fs::Permissions::from_mode(mode.into()))
            .map_err(|error| miette!(error))
            .wrap_err("cannot apply target mode")?;
    }
    Ok(if existed || replaced {
        EntryResult::Replaced
    } else {
        EntryResult::Deployed
    })
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter)]
pub enum ObstructionChoice {
    Skip,
    ViewDiff,
    Replace,
    ReplaceAll,
}

impl PromptOption for ObstructionChoice {
    fn hotkey(&self) -> Option<char> {
        match self {
            Self::ReplaceAll => Some('a'),
            _ => None,
        }
    }
}

#[cfg(any(test, feature = "testing"))]
thread_local! {
    pub static PROMPT_CHOICE: RefCell<Option<ObstructionChoice>> = const { RefCell::new(None) };
    pub static PROMPT_COUNT: RefCell<usize> = const { RefCell::new(0) };
}

#[cfg(any(test, feature = "testing"))]
pub fn set_prompt_choice(choice: ObstructionChoice) {
    PROMPT_CHOICE.with(|current| *current.borrow_mut() = Some(choice));
}

fn prompt_for_obstruction(
    entry: &config::DeploymentEntry,
    obstruction: &Path,
) -> std::result::Result<ObstructionChoice, PromptError> {
    #[cfg(any(test, feature = "testing"))]
    if PROMPT_CHOICE.with(|choice| choice.borrow().is_some()) {
        PROMPT_COUNT.with_borrow_mut(|count| *count += 1);
        return Ok(PROMPT_CHOICE.with(|choice| choice.borrow().to_owned().unwrap()));
    }

    println!(
        r#"Cannot deploy {} {},
{} {} is already present."#,
        path_kind(&entry.source_path)?,
        entry.source_path.display(),
        path_kind(obstruction)?,
        obstruction.display()
    );
    let question = "How would you like to proceed?";
    let should_show_diff = fs::metadata(&entry.source_path)
        .is_ok_and(|metadata| metadata.is_file())
        && fs::metadata(obstruction).is_ok_and(|metadata| metadata.is_file());
    tui::prompt::SelectPrompt::new()
        .question(question)
        .filter(move |choice| should_show_diff || *choice != ObstructionChoice::ViewDiff)
        .interact()
}

fn path_kind(path: &Path) -> std::io::Result<&'static str> {
    Ok(if fs::symlink_metadata(path)?.is_dir() {
        "directory"
    } else {
        "file"
    })
}

fn show_diff(
    entry: &config::DeploymentEntry,
    target: &Path,
    context: &std::collections::HashMap<String, templater::value::Value>,
) -> Result<()> {
    let source_bytes = if entry.deploy_type == DeployType::Template {
        template::render_template(&entry.source_path, context)?
    } else {
        fs::read(&entry.source_path).map_err(|error| miette!(error))?
    };
    let target_bytes = fs::read(target).map_err(|error| miette!(error))?;
    let source_lossy = String::from_utf8_lossy(&source_bytes);
    let target_lossy = String::from_utf8_lossy(&target_bytes);
    let text_diff = TextDiff::from_lines(&target_lossy, &source_lossy);
    let mut diff = text_diff.unified_diff();
    diff.header(
        &target.display().to_string(),
        &entry.source_path.display().to_string(),
    );
    std::io::stdout().flush().map_err(|error| miette!(error))?;

    enum PagerResolution<'a> {
        DotriftPager(&'a str),
        Pager(&'a str),
        Stdout,
    }

    let dotrift_pager = std::env::var("DOTRIFT_PAGER").ok();
    let pager = std::env::var("PAGER").ok();
    let resolution = match dotrift_pager.as_deref() {
        Some(command) if !command.trim().is_empty() => PagerResolution::DotriftPager(command),
        _ => match pager.as_deref() {
            Some(command) if !command.trim().is_empty() => PagerResolution::Pager(command),
            _ => PagerResolution::Stdout,
        },
    };
    match resolution {
        PagerResolution::DotriftPager(command) => run_pager(command, &diff)
            .map_err(|error| miette!(error).wrap_err("cannot run DOTRIFT_PAGER")),
        PagerResolution::Pager(command) => {
            if run_pager(command, &diff).is_err() {
                println!("{diff}");
            }
            Ok(())
        }
        PagerResolution::Stdout => {
            println!("{diff}");
            Ok(())
        }
    }
}

fn run_pager(command: &str, diff: &dyn Display) -> std::io::Result<()> {
    let mut parts = command.split_whitespace();
    let program = parts.next().expect("non-empty pager command");
    let mut child = Command::new(program)
        .args(parts)
        .stdin(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_fmt(format_args!("{diff}"))?;
    child.wait()?;
    Ok(())
}

fn remove_path(database: &StateDatabase, path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| miette!(error))?;
    if metadata.file_type().is_dir() {
        let mut children = fs::read_dir(path)
            .map_err(|error| miette!(error))?
            .map(|entry| {
                entry
                    .map(|entry| entry.path())
                    .map_err(|error| miette!(error))
            })
            .collect::<Result<Vec<_>>>()?;
        children.sort();
        for child in children {
            remove_path(database, &child)?;
        }
        fs::remove_dir(path).map_err(|error| miette!(error))?;
    } else {
        fs::remove_file(path).map_err(|error| miette!(error))?;
    }
    database.remove(path)?;
    Ok(())
}

fn parent_obstruction(
    target_root: &Path,
    target_path: &Path,
) -> Result<Option<std::path::PathBuf>> {
    let parent = target_path
        .parent()
        .ok_or_else(|| miette!("target path has no parent"))?;
    let relative = parent
        .strip_prefix(target_root)
        .map_err(|_| miette!("target path is outside target directory"))?;
    let mut current = target_root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => return Ok(Some(current)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(miette!(error).wrap_err(format!(
                    "cannot inspect target parent `{}`",
                    current.display()
                )));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    use super::*;
    use tempfile::tempdir;
    use test_case::test_case;

    #[test_case(|t| t.join("file") => None; "direct_child_has_no_obstruction")]
    #[test_case(|t| {
        fs::create_dir_all(t.join("a/b/c")).unwrap();
        t.join("a/b/c/file")
    } => None; "all_parent_dirs_are_clear")]
    #[test_case(|t| {
        fs::create_dir(t.join("a")).unwrap();
        fs::write(t.join("a/b"), b"").unwrap();
        t.join("a/b/c/file")
    } => Some(PathBuf::from("a/b")); "intermediate_parent_is_a_file")]
    #[test_case(|t| {
        fs::create_dir(t.join("x")).unwrap();
        symlink(t.join("x"), t.join("a")).unwrap();
        t.join("a/b/file")
    } => Some(PathBuf::from("a")); "symlink_to_dir_is_an_obstruction")]
    #[test_case(|t| {
        fs::create_dir(t.join("a")).unwrap();
        t.join("a/b/c/file")
    } => None; "missing_parent_subtree_short_circuits")]
    #[test_case(|t| t.join("a/b/file") => None; "empty_root_target_has_missing_parent")]
    #[test_case(|_| PathBuf::from("/outside/target/file") => panics ""; "target_outside_root")]
    #[test_case(|_| PathBuf::from("/") => panics ""; "target_has_no_parent")]
    fn parent_obstruction_test<F: Fn(&Path) -> PathBuf>(setup: F) -> Option<PathBuf> {
        let tmp = tempdir().unwrap();
        parent_obstruction(tmp.path(), &setup(tmp.path()))
            .unwrap()
            .map(|obstruction| obstruction.strip_prefix(tmp.path()).unwrap().to_path_buf())
    }

    #[test_case(
        |db, t| {
            fs::write(t.join("file"), b"data").unwrap();
            db.put(&crate::record!(f, t.join("file"), "src", hash::hash_bytes(b"data"))).unwrap();
            t.join("file")
        },
        |db, t| {
            assert!(fs::symlink_metadata(t.join("file")).is_err());
            assert_eq!(db.record(&t.join("file")).unwrap(), None);
        };
        "removes_regular_file"
    )]
    #[test_case(
        |db, t| {
            fs::write(t.join("real"), b"data").unwrap();
            symlink(t.join("real"), t.join("link")).unwrap();
            db.put(&crate::record!(s, t.join("link"), "src", t.join("real"))).unwrap();
            t.join("link")
        },
        |db, t| {
            assert!(fs::symlink_metadata(t.join("link")).is_err());
            assert!(fs::symlink_metadata(t.join("real")).is_ok());
            assert_eq!(db.record(&t.join("link")).unwrap(), None);
        };
        "removes_symlink_but_not_target"
    )]
    #[test_case(
        |_db, t| {
            fs::create_dir(t.join("target")).unwrap();
            symlink(t.join("target"), t.join("link")).unwrap();
            t.join("link")
        },
        |_db, t| {
            assert!(fs::symlink_metadata(t.join("link")).is_err());
            assert!(fs::symlink_metadata(t.join("target")).is_ok());
        };
        "removes_symlink_to_dir_without_following"
    )]
    #[test_case(
        |db, t| {
            fs::create_dir_all(t.join("a/b/c")).unwrap();
            fs::write(t.join("a/b/c/deep"), b"").unwrap();
            db.put(&crate::record!(f, t.join("a"), "src/a", "h1")).unwrap();
            db.put(&crate::record!(f, t.join("a/b/c"), "src/c", "h2")).unwrap();
            db.put(&crate::record!(f, t.join("a/b/c/deep"), "src/deep", "h3")).unwrap();
            t.join("a")
        },
        |db, t| {
            assert!(fs::symlink_metadata(t.join("a")).is_err());
            for relative in ["a", "a/b/c", "a/b/c/deep"] {
                assert_eq!(db.record(&t.join(relative)).unwrap(), None);
            }
        };
        "removes_nested_directory_tree"
    )]
    #[test_case(
        |db, t| {
            fs::create_dir(t.join("a")).unwrap();
            fs::write(t.join("real"), b"").unwrap();
            symlink(t.join("real"), t.join("a/link")).unwrap();
            db.put(&crate::record!(f, t.join("a"), "src/a", "h1")).unwrap();
            db.put(&crate::record!(s, t.join("a/link"), "src/link", t.join("real"))).unwrap();
            t.join("a")
        },
        |db, t| {
            assert!(fs::symlink_metadata(t.join("a")).is_err());
            assert!(fs::symlink_metadata(t.join("real")).is_ok());
            assert_eq!(db.record(&t.join("a")).unwrap(), None);
            assert_eq!(db.record(&t.join("a/link")).unwrap(), None);
        };
        "removes_tree_containing_symlink_without_following"
    )]
    #[test_case(
        |db, t| {
            fs::create_dir_all(t.join("a")).unwrap();
            fs::write(t.join("a/child"), b"").unwrap();
            fs::write(t.join("other"), b"").unwrap();
            db.put(&crate::record!(f, t.join("a"), "src/a", "h1")).unwrap();
            t.join("a")
        },
        |db, t| {
            assert!(fs::symlink_metadata(t.join("a")).is_err());
            assert!(fs::symlink_metadata(t.join("other")).is_ok());
            assert_eq!(db.record(&t.join("a")).unwrap(), None);
        };
        "removes_tree_but_preserves_sibling"
    )]
    #[test_case(
        |_db, t| {
            fs::create_dir(t.join("empty")).unwrap();
            t.join("empty")
        },
        |_db, t| assert!(fs::symlink_metadata(t.join("empty")).is_err());
        "removes_empty_directory"
    )]
    #[test_case(
        |_db, t| t.join("missing"),
        |_db, _t| {}
        => panics ""
        ; "missing_path_is_an_error"
    )]
    fn remove_path_test<
        F: Fn(&mut StateDatabase, &Path) -> PathBuf,
        G: Fn(&StateDatabase, &Path),
    >(
        setup: F,
        assert: G,
    ) {
        let tmp = tempdir().unwrap();
        let mut database = StateDatabase::open_at(&tmp.path().join("db")).unwrap();
        let path = setup(&mut database, tmp.path());
        remove_path(&database, &path).unwrap();
        assert(&database, tmp.path())
    }

    macro_rules! entry {
        ($source:expr, $target:expr, $deploy_type:expr) => {
            config::DeploymentEntry {
                source_path: $source,
                target_path: $target,
                deploy_type: $deploy_type,
                mode: None,
            }
        };
        ($source:expr, $target:expr, $deploy_type:expr, $mode:expr) => {
            config::DeploymentEntry {
                source_path: $source,
                target_path: $target,
                deploy_type: $deploy_type,
                mode: $mode,
            }
        };
    }

    #[test_case(
        |_db, t| {
            let source = t.join("src");
            fs::write(&source, b"content").unwrap();
            entry!(source, t.join("target"), DeployType::Symlink)
        },
        |db, t| {
            let source = t.join("src");
            let metadata = fs::symlink_metadata(t.join("target")).unwrap();
            assert!(metadata.file_type().is_symlink());
            assert_eq!(fs::read_link(t.join("target")).unwrap(), source);
            let record = db.record(&t.join("target")).unwrap().unwrap();
            assert_eq!(record.kind, Kind::Symlink);
            assert_eq!(record.source_path, source);
            assert_eq!(record.link_target, Some(source));
            assert_eq!(record.content_hash, None);
        }
        => EntryResult::Deployed
        ; "deploys_symlink"
    )]
    #[test_case(
        |_db, t| {
            let source = t.join("src");
            fs::write(&source, b"copy").unwrap();
            entry!(source, t.join("target"), DeployType::Copy)
        },
        |db, t| {
            assert_eq!(fs::read(t.join("target")).unwrap(), b"copy");
            let metadata = fs::symlink_metadata(t.join("target")).unwrap();
            assert!(metadata.file_type().is_file());
            let record = db.record(&t.join("target")).unwrap().unwrap();
            assert_eq!(record.kind, Kind::File);
            assert_eq!(record.content_hash, Some(hash::hash_bytes(b"copy")));
            assert_eq!(record.link_target, None);
        }
        => EntryResult::Deployed
        ; "deploys_copy"
    )]
    #[test_case(
        |_db, t| {
            let source = t.join("src");
            fs::write(&source, br#"{{"rendered"}}"#).unwrap();
            entry!(source, t.join("target"), DeployType::Template)
        },
        |db, t| {
            assert_eq!(fs::read(t.join("target")).unwrap(), b"rendered");
            let metadata = fs::symlink_metadata(t.join("target")).unwrap();
            assert!(metadata.file_type().is_file());
            let record = db.record(&t.join("target")).unwrap().unwrap();
            assert_eq!(record.kind, Kind::File);
            assert_eq!(record.content_hash, Some(hash::hash_bytes(b"rendered")));
        }
        => EntryResult::Deployed
        ; "deploys_template"
    )]
    #[test_case(
        |_db, t| {
            let source = t.join("src");
            fs::write(&source, b"copy").unwrap();
            let mode = config::DeployMode::try_from(0o600).unwrap();
            entry!(source, t.join("target"), DeployType::Copy, Some(mode))
        },
        |_db, t| {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::read(t.join("target")).unwrap(), b"copy");
            let mode = fs::metadata(t.join("target")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        => EntryResult::Deployed
        ; "deploys_copy_with_mode"
    )]
    #[test_case(
        |_db, t| {
            let source = t.join("src");
            fs::write(&source, b"deep").unwrap();
            entry!(source, t.join("a/b/c/file"), DeployType::Copy)
        },
        |db, t| {
            assert_eq!(fs::read(t.join("a/b/c/file")).unwrap(), b"deep");
            assert!(db.record(&t.join("a/b/c/file")).unwrap().is_some());
            assert_eq!(db.managed_paths().iter().len(), 1);
        }
        => EntryResult::Deployed
        ; "deploys_into_nested_missing_dirs"
    )]
    #[test_case(
        |db, t| {
            let target = t.join("target");
            fs::write(&target, b"old").unwrap();
            db.put(&crate::record!(f, target.clone(), "old-src", hash::hash_bytes(b"old"))).unwrap();
            let source = t.join("src");
            fs::write(&source, b"new").unwrap();
            entry!(source, target, DeployType::Copy)
        },
        |db, t| {
            assert_eq!(fs::read(t.join("target")).unwrap(), b"new");
            let record = db.record(&t.join("target")).unwrap().unwrap();
            assert_eq!(record.content_hash, Some(hash::hash_bytes(b"new")));
        }
        => EntryResult::Replaced
        ; "replaces_managed_file_target"
    )]
    #[test_case(
        |db, t| {
            let target = t.join("target");
            let old_source = t.join("old-src");
            fs::write(&old_source, b"old").unwrap();
            symlink(&old_source, &target).unwrap();
            db.put(&crate::record!(s, target.clone(), "old-src", old_source.clone())).unwrap();
            let source = t.join("src");
            fs::write(&source, b"new").unwrap();
            entry!(source, target, DeployType::Symlink)
        },
        |db, t| {
            assert_eq!(fs::read_link(t.join("target")).unwrap(), t.join("src"));
            let record = db.record(&t.join("target")).unwrap().unwrap();
            assert_eq!(record.kind, Kind::Symlink);
            assert_eq!(record.source_path, t.join("src"));
            assert_eq!(record.link_target, Some(t.join("src")));
        }
        => EntryResult::Replaced
        ; "replaces_managed_symlink_target"
    )]
    #[test_case(
        |_db, t| {
            set_prompt_choice(ObstructionChoice::Replace);
            let target = t.join("target");
            fs::write(&target, b"old").unwrap();
            let source = t.join("src");
            fs::write(&source, b"new").unwrap();
            entry!(source, target, DeployType::Copy)
        },
        |db, t| {
            assert_eq!(fs::read(t.join("target")).unwrap(), b"new");
            assert!(db.record(&t.join("target")).unwrap().is_some());
        }
        => EntryResult::Replaced
        ; "replaces_unmanaged_target_via_prompt"
    )]
    #[test_case(
        |_db, t| {
            set_prompt_choice(ObstructionChoice::Skip);
            let target = t.join("target");
            fs::write(&target, b"old").unwrap();
            let source = t.join("src");
            fs::write(&source, b"new").unwrap();
            entry!(source, target, DeployType::Copy)
        },
        |db, t| {
            assert_eq!(fs::read(t.join("target")).unwrap(), b"old");
            assert_eq!(db.record(&t.join("target")).unwrap(), None);
        }
        => EntryResult::Skipped
        ; "skips_unmanaged_target_via_prompt"
    )]
    #[test_case(
        |_db, t| {
            set_prompt_choice(ObstructionChoice::Skip);
            fs::create_dir(t.join("a")).unwrap();
            fs::write(t.join("a/b"), b"").unwrap();
            let source = t.join("src");
            fs::write(&source, b"new").unwrap();
            entry!(source, t.join("a/b/file"), DeployType::Copy)
        },
        |db, t| {
            assert!(fs::symlink_metadata(t.join("a/b")).unwrap().is_file());
            assert!(fs::symlink_metadata(t.join("a/b/file")).is_err());
            assert_eq!(db.record(&t.join("a/b/file")).unwrap(), None);
        }
        => EntryResult::Skipped
        ; "skips_parent_obstruction_via_prompt"
    )]
    #[test_case(
        |_db, t| {
            set_prompt_choice(ObstructionChoice::Replace);
            fs::create_dir(t.join("a")).unwrap();
            fs::write(t.join("a/b"), b"").unwrap();
            let source = t.join("src");
            fs::write(&source, b"new").unwrap();
            entry!(source, t.join("a/b/file"), DeployType::Copy)
        },
        |db, t| {
            assert!(fs::metadata(t.join("a/b")).unwrap().is_dir());
            assert_eq!(fs::read(t.join("a/b/file")).unwrap(), b"new");
            assert!(db.record(&t.join("a/b/file")).unwrap().is_some());
        }
        => EntryResult::Replaced
        ; "removes_parent_obstruction_via_prompt"
    )]
    #[test_case(
        |_db, t| {
            set_prompt_choice(ObstructionChoice::Replace);
            fs::create_dir_all(t.join("a")).unwrap();
            fs::create_dir(t.join("real")).unwrap();
            symlink(t.join("real"), t.join("a/b")).unwrap();
            let source = t.join("src");
            fs::write(&source, b"new").unwrap();
            entry!(source, t.join("a/b/file"), DeployType::Copy)
        },
        |db, t| {
            assert!(fs::metadata(t.join("a/b")).unwrap().is_dir());
            assert_eq!(fs::read(t.join("a/b/file")).unwrap(), b"new");
            assert!(db.record(&t.join("a/b/file")).unwrap().is_some());
        }
        => EntryResult::Replaced
        ; "removes_symlink_parent_obstruction_via_prompt"
    )]
    #[test_case(
        |_db, t| {
            set_prompt_choice(ObstructionChoice::Replace);
            fs::create_dir_all(t.join("a")).unwrap();
            fs::create_dir(t.join("real")).unwrap();
            fs::write(t.join("real/file"), b"old").unwrap();
            symlink(t.join("real"), t.join("a/b")).unwrap();
            let source = t.join("src");
            fs::write(&source, b"new").unwrap();
            entry!(source, t.join("a/b/file"), DeployType::Copy)
        },
        |db, t| {
            assert!(fs::metadata(t.join("a/b")).unwrap().is_dir());
            assert_eq!(fs::read(t.join("a/b/file")).unwrap(), b"new");
            assert_eq!(fs::read(t.join("real/file")).unwrap(), b"old");
            assert!(db.record(&t.join("a/b/file")).unwrap().is_some());
        }
        => EntryResult::Replaced
        ; "symlink_parent_with_existing_leaf_is_replaced_as_obstruction"
    )]
    #[test_case(
        |_db, t| {
            let source = t.join("src");
            symlink(t.join("missing"), &source).unwrap();
            entry!(source, t.join("target"), DeployType::Copy)
        },
        |db, t| {
            assert!(fs::symlink_metadata(t.join("target")).is_err());
            assert_eq!(db.record(&t.join("target")).unwrap(), None);
        }
        => panics ""
        ; "source_is_a_dangling_symlink"
    )]
    #[test_case(
        |_db, t| {
            let source = t.join("srcdir");
            fs::create_dir(&source).unwrap();
            entry!(source, t.join("target"), DeployType::Copy)
        },
        |db, t| {
            assert!(fs::symlink_metadata(t.join("target")).is_err());
            assert_eq!(db.record(&t.join("target")).unwrap(), None);
        }
        => panics ""
        ; "source_is_not_a_regular_file"
    )]
    #[test_case(
        |_db, t| {
            let source = t.join("src");
            fs::write(&source, b"{{ missing }}").unwrap();
            entry!(source, t.join("target"), DeployType::Template)
        },
        |db, t| {
            assert!(fs::symlink_metadata(t.join("target")).is_err());
            assert_eq!(db.record(&t.join("target")).unwrap(), None);
        }
        => panics ""
        ; "template_with_undefined_variable_errors"
    )]
    fn deploy_entry_test<
        F: Fn(&mut StateDatabase, &Path) -> config::DeploymentEntry,
        G: Fn(&StateDatabase, &Path),
    >(
        setup: F,
        assert: G,
    ) -> EntryResult {
        let tmp = tempdir().unwrap();
        let mut database = StateDatabase::open_at(&tmp.path().join("db")).unwrap();
        let entry = setup(&mut database, tmp.path());
        let mut replace_all = false;
        let result = deploy_entry(
            &database,
            tmp.path(),
            &entry,
            &HashMap::new(),
            &mut replace_all,
        );
        assert(&database, tmp.path());
        result.unwrap()
    }

    #[test_case(
        |_db, t| {
            let target_a = t.join("target_a");
            fs::write(&target_a, b"old-a").unwrap();
            let source_a = t.join("src_a");
            fs::write(&source_a, b"new-a").unwrap();
            let target_b = t.join("target_b");
            fs::write(&target_b, b"old-b").unwrap();
            let source_b = t.join("src_b");
            fs::write(&source_b, b"new-b").unwrap();
            vec![
                entry!(source_a, target_a, DeployType::Copy),
                entry!(source_b, target_b, DeployType::Copy),
            ]
        },
        |db, t| {
            assert_eq!(fs::read(t.join("target_a")).unwrap(), b"new-a");
            assert_eq!(fs::read(t.join("target_b")).unwrap(), b"new-b");
            assert!(db.record(&t.join("target_a")).unwrap().is_some());
            assert!(db.record(&t.join("target_b")).unwrap().is_some());
        }
        => vec![EntryResult::Replaced, EntryResult::Replaced]
        ; "replace_all_latches_across_entries"
    )]
    #[test_case(
        |_db, t| {
            fs::create_dir(t.join("a")).unwrap();
            fs::write(t.join("a/b"), b"").unwrap();
            let source = t.join("src");
            fs::write(&source, b"new").unwrap();
            let target_b = t.join("target_b");
            fs::write(&target_b, b"old-b").unwrap();
            let source_b = t.join("src_b");
            fs::write(&source_b, b"new-b").unwrap();
            vec![
                entry!(source, t.join("a/b/file"), DeployType::Copy),
                entry!(source_b, target_b, DeployType::Copy),
            ]
        },
        |db, t| {
            assert!(fs::metadata(t.join("a/b")).unwrap().is_dir());
            assert_eq!(fs::read(t.join("a/b/file")).unwrap(), b"new");
            assert_eq!(fs::read(t.join("target_b")).unwrap(), b"new-b");
            assert!(db.record(&t.join("a/b/file")).unwrap().is_some());
            assert!(db.record(&t.join("target_b")).unwrap().is_some());
        }
        => vec![EntryResult::Replaced, EntryResult::Replaced]
        ; "replace_all_latches_from_parent_obstruction"
    )]
    #[test_case(
        |_db, t| {
            let target_a = t.join("target_a");
            fs::write(&target_a, b"old-a").unwrap();
            let source_a = t.join("src_a");
            fs::write(&source_a, b"new-a").unwrap();
            fs::create_dir(t.join("a")).unwrap();
            fs::write(t.join("a/b"), b"").unwrap();
            let source_b = t.join("src_b");
            fs::write(&source_b, b"new-b").unwrap();
            vec![
                entry!(source_a, target_a, DeployType::Copy),
                entry!(source_b, t.join("a/b/file"), DeployType::Copy),
            ]
        },
        |db, t| {
            assert_eq!(fs::read(t.join("target_a")).unwrap(), b"new-a");
            assert!(fs::metadata(t.join("a/b")).unwrap().is_dir());
            assert_eq!(fs::read(t.join("a/b/file")).unwrap(), b"new-b");
            assert!(db.record(&t.join("target_a")).unwrap().is_some());
            assert!(db.record(&t.join("a/b/file")).unwrap().is_some());
        }
        => vec![EntryResult::Replaced, EntryResult::Replaced]
        ; "replace_all_latches_via_target_prompt_then_removes_parent_obstruction_without_prompt"
    )]
    fn deploy_entry_replace_all_test<
        F: Fn(&mut StateDatabase, &Path) -> Vec<config::DeploymentEntry>,
        G: Fn(&StateDatabase, &Path),
    >(
        setup: F,
        assert: G,
    ) -> Vec<EntryResult> {
        let tmp = tempdir().unwrap();
        let mut database = StateDatabase::open_at(&tmp.path().join("db")).unwrap();
        set_prompt_choice(ObstructionChoice::ReplaceAll);
        let entries = setup(&mut database, tmp.path());
        let mut replace_all = false;
        let mut results = Vec::new();
        for entry in &entries {
            let result = deploy_entry(
                &database,
                tmp.path(),
                entry,
                &HashMap::new(),
                &mut replace_all,
            );
            results.push(result.unwrap());
        }
        assert(&database, tmp.path());
        assert_eq!(PROMPT_COUNT.with(|c| *c.borrow()), 1);
        results
    }
}
