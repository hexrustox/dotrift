use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use miette::{Result, WrapErr, miette};
use similar::TextDiff;
use tui::prompt::{ObstructionChoice, SelectPrompt};

use crate::config::{self, DeployType};
use crate::hash;
use crate::managed;
use crate::state::{Kind, StateDatabase, StateLock, StateRecord};

/// Reconciles the desired deployment with the target directory.
pub fn run(source: &Path, target_override: Option<std::path::PathBuf>) -> Result<()> {
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
        return Ok(());
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
        }
    }
    println!("deployed {deployed}, replaced {replaced}, skipped {skipped}");
    if skipped > 0 {
        return Err(miette!("one or more obstructions were skipped"));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum EntryResult {
    Deployed,
    Replaced,
    Skipped,
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
                match prompt_for_obstruction(entry, &entry.target_path, context)? {
                    ObstructionChoice::Skip => return Ok(EntryResult::Skipped),
                    ObstructionChoice::ViewDetail => {
                        show_detail(entry, &entry.target_path, context)?
                    }
                    ObstructionChoice::Replace => {
                        remove_path(database, &entry.target_path)?;
                        replaced = true;
                        break;
                    }
                    ObstructionChoice::ReplaceAll => {
                        *replace_all = true;
                        remove_path(database, &entry.target_path)?;
                        replaced = true;
                        break;
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
                match prompt_for_obstruction(entry, &obstruction, context)? {
                    ObstructionChoice::Skip => return Ok(EntryResult::Skipped),
                    ObstructionChoice::ViewDetail => show_detail(entry, &obstruction, context)?,
                    ObstructionChoice::Replace => {
                        remove_path(database, &obstruction)?;
                        replaced = true;
                        break;
                    }
                    ObstructionChoice::ReplaceAll => {
                        *replace_all = true;
                        remove_path(database, &obstruction)?;
                        replaced = true;
                        break;
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
                config::render_template(&entry.source_path, context)?
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

fn prompt_for_obstruction(
    entry: &config::DeploymentEntry,
    obstruction: &Path,
    _context: &std::collections::HashMap<String, templater::value::Value>,
) -> Result<ObstructionChoice> {
    let can_show_detail = fs::metadata(&entry.source_path).is_ok_and(|metadata| metadata.is_file())
        && fs::metadata(obstruction).is_ok_and(|metadata| metadata.is_file());
    let options = if can_show_detail {
        vec![
            ObstructionChoice::Skip,
            ObstructionChoice::ViewDetail,
            ObstructionChoice::Replace,
            ObstructionChoice::ReplaceAll,
        ]
    } else {
        vec![
            ObstructionChoice::Skip,
            ObstructionChoice::Replace,
            ObstructionChoice::ReplaceAll,
        ]
    };
    let question = format!(
        "obstruction\nsource: {}\ntarget: {}\n{}\n{}",
        describe_path(&entry.source_path)?,
        describe_path(obstruction)?,
        entry.source_path.display(),
        obstruction.display()
    );
    SelectPrompt::new()
        .question(question)
        .options(options)
        .interact()
        .map_err(|error| miette!("obstruction prompt failed: {error:?}"))
}

fn describe_path(path: &Path) -> Result<String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| miette!(error).wrap_err(format!("cannot inspect `{}`", path.display())))?;
    let kind = if metadata.file_type().is_symlink() {
        let target = fs::read_link(path).map_err(|error| miette!(error))?;
        format!("symlink -> {}", target.display())
    } else if metadata.file_type().is_file() {
        format!(
            "regular file, {} bytes, modified {:?}",
            metadata.len(),
            metadata.modified().ok()
        )
    } else if metadata.file_type().is_dir() {
        let count = fs::read_dir(path).map_err(|error| miette!(error))?.count();
        format!("directory, {count} entries")
    } else {
        "other".to_string()
    };
    Ok(format!("{}: {kind}", path.display()))
}

fn show_detail(
    entry: &config::DeploymentEntry,
    target: &Path,
    context: &std::collections::HashMap<String, templater::value::Value>,
) -> Result<()> {
    let source = if entry.deploy_type == DeployType::Template {
        config::render_template(&entry.source_path, context)?
    } else {
        fs::read(&entry.source_path).map_err(|error| miette!(error))?
    };
    let target = fs::read(target).map_err(|error| miette!(error))?;
    println!(
        "{}",
        TextDiff::from_lines(
            &String::from_utf8_lossy(&source),
            &String::from_utf8_lossy(&target),
        )
        .unified_diff()
        .header("source", "target")
    );
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
