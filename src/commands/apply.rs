// TODO optimize fs::read
use std::{
    fmt::Display,
    fs,
    io::Write,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    process::{Command, Stdio},
};

use miette::{Result, WrapErr, miette};
use similar::TextDiff;
use strum::EnumIter;
use tui::prompt::{PromptError, PromptOption, SelectPrompt};

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

#[derive(Clone, Copy)]
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

    let existed = match fs::symlink_metadata(&entry.target_path) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(miette!(error).wrap_err(format!(
                "cannot inspect target `{}`",
                entry.target_path.display()
            )));
        }
    };
    let mut replaced = false;
    if existed {
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
                match prompt_for_obstruction(entry, &entry.target_path, context) {
                    Ok(ObstructionChoice::Skip) => return Ok(EntryResult::Skipped),
                    Ok(ObstructionChoice::ViewDetail) => {
                        show_detail(entry, &entry.target_path, context)?
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

    if let Some(obstruction) = parent_obstruction(target_root, &entry.target_path)? {
        if !*replace_all {
            loop {
                match prompt_for_obstruction(entry, &obstruction, context) {
                    Ok(ObstructionChoice::Skip) => return Ok(EntryResult::Skipped),
                    Ok(ObstructionChoice::ViewDetail) => show_detail(entry, &obstruction, context)?,
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
    }
    create_parent_dirs(target_root, &entry.target_path)?;

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
enum ObstructionChoice {
    Skip,
    ViewDetail,
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

fn prompt_for_obstruction(
    entry: &config::DeploymentEntry,
    obstruction: &Path,
    _context: &std::collections::HashMap<String, templater::value::Value>,
) -> std::result::Result<ObstructionChoice, PromptError> {
    let can_show_detail = fs::metadata(&entry.source_path).is_ok_and(|metadata| metadata.is_file())
        && fs::metadata(obstruction).is_ok_and(|metadata| metadata.is_file());
    let question = format!(
        r#"Cannot deploy {} {}:
{} {} is already present.
How would you like to proceed?"#,
        path_kind(&entry.source_path)?,
        entry.source_path.display(),
        path_kind(obstruction)?,
        obstruction.display()
    );
    SelectPrompt::new()
        .question(question)
        .filter(move |choice| can_show_detail || *choice != ObstructionChoice::ViewDetail)
        .interact()
}

fn path_kind(path: &Path) -> std::io::Result<&'static str> {
    Ok(if fs::symlink_metadata(path)?.is_dir() {
        "directory"
    } else {
        "file"
    })
}

fn show_detail(
    entry: &config::DeploymentEntry,
    target: &Path,
    context: &std::collections::HashMap<String, templater::value::Value>,
) -> Result<()> {
    let source = if entry.deploy_type == DeployType::Template {
        template::render_template(&entry.source_path, context)?
    } else {
        fs::read(&entry.source_path).map_err(|error| miette!(error))?
    };
    let target = fs::read(target).map_err(|error| miette!(error))?;
    let source_lossy = String::from_utf8_lossy(&source);
    let target_lossy = String::from_utf8_lossy(&target);
    let text_diff = TextDiff::from_lines(&source_lossy, &target_lossy);
    let mut diff = text_diff.unified_diff();
    diff.header("source", "target");
    std::io::stdout().flush().map_err(|error| miette!(error))?;
    display_detail(
        &diff,
        std::env::var("DOTRIFT_PAGER").ok().as_deref(),
        std::env::var("PAGER").ok().as_deref(),
    )
}

enum PagerResolution<'a> {
    DotriftPager(&'a str),
    Pager(&'a str),
    Stdout,
}

fn resolve_pager<'a>(
    dotrift_pager: Option<&'a str>,
    pager: Option<&'a str>,
) -> PagerResolution<'a> {
    match dotrift_pager {
        Some(command) if !command.trim().is_empty() => PagerResolution::DotriftPager(command),
        _ => match pager {
            Some(command) if !command.trim().is_empty() => PagerResolution::Pager(command),
            _ => PagerResolution::Stdout,
        },
    }
}

fn display_detail(
    diff: &dyn Display,
    dotrift_pager: Option<&str>,
    pager: Option<&str>,
) -> Result<()> {
    match resolve_pager(dotrift_pager, pager) {
        PagerResolution::DotriftPager(command) => run_pager(command, diff)
            .map_err(|error| miette!(error).wrap_err("cannot run DOTRIFT_PAGER")),
        PagerResolution::Pager(command) => {
            if run_pager(command, diff).is_err() {
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

fn create_parent_dirs(target_root: &Path, target_path: &Path) -> Result<()> {
    let parent = target_path
        .parent()
        .ok_or_else(|| miette!("target path has no parent"))?;
    let relative = parent
        .strip_prefix(target_root)
        .map_err(|_| miette!("target path is outside target directory"))?;
    let mut current = target_root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        if fs::symlink_metadata(&current).is_err() {
            fs::create_dir(&current)
                .map_err(|error| miette!(error))
                .map_err(|error| {
                    miette!(error).wrap_err(format!(
                        "cannot create target parent `{}`",
                        current.display()
                    ))
                })?;
        }
    }
    Ok(())
}
