use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use miette::{Result, miette};

use crate::config::{self, DeployType};
use crate::managed;
use crate::state::{Kind, StateDatabase, StateLock, StateRecord};

/// Reconciles the desired symlink deployment with the target directory.
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
    if !target.exists() {
        fs::create_dir_all(target)
            .map_err(|error| miette!(error))
            .map_err(|error| miette!(error).wrap_err("cannot create target directory"))?;
    }

    let database = StateDatabase::open()?;
    let mut entries = deployment.entries;
    entries.sort_by(|left, right| left.target_path.cmp(&right.target_path));
    for entry in entries {
        deploy_entry(&database, target, &entry)?;
    }
    Ok(())
}

fn deploy_entry(
    database: &StateDatabase,
    target_root: &Path,
    entry: &config::DeploymentEntry,
) -> Result<()> {
    if entry.deploy_type != DeployType::Symlink {
        return Err(miette!(
            "deploy type `{}` is not supported by basic apply",
            match entry.deploy_type {
                DeployType::Copy => "copy",
                DeployType::Template => "template",
                DeployType::Symlink => "symlink",
            }
        ));
    }
    if !fs::metadata(&entry.source_path)
        .map_err(|error| miette!(error))?
        .is_file()
    {
        return Err(miette!(
            "source path `{}` is no longer a regular file",
            entry.source_path.display()
        ));
    }

    let old_record = database.record(&entry.target_path)?;
    match fs::symlink_metadata(&entry.target_path) {
        Ok(metadata) => {
            let managed = old_record
                .as_ref()
                .map(managed::is_managed)
                .transpose()?
                .unwrap_or(false);
            if !managed {
                return Err(miette!(
                    "target path `{}` is obstructed",
                    entry.target_path.display()
                ));
            }
            if metadata.file_type().is_dir() {
                return Err(miette!(
                    "target path `{}` is a directory",
                    entry.target_path.display()
                ));
            }
            fs::remove_file(&entry.target_path)
                .map_err(|error| miette!(error))
                .map_err(|error| miette!(error).wrap_err("cannot remove managed target"))?;
            database.remove(&entry.target_path)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // A missing target is deployable even when its old state record remains.
        }
        Err(error) => {
            return Err(miette!(error).wrap_err(format!(
                "cannot inspect target `{}`",
                entry.target_path.display()
            )));
        }
    }

    create_parent_dirs(target_root, &entry.target_path)?;
    symlink(&entry.source_path, &entry.target_path)
        .map_err(|error| miette!(error))
        .map_err(|error| miette!(error).wrap_err("cannot create target symlink"))?;
    database.put(&StateRecord {
        target_path: entry.target_path.clone(),
        source_path: entry.source_path.clone(),
        kind: Kind::Symlink,
        link_target: Some(entry.source_path.clone()),
        content_hash: None,
    })?;
    Ok(())
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
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => {
                return Err(miette!(
                    "target parent `{}` is an obstruction",
                    current.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current)
                    .map_err(|error| miette!(error))
                    .map_err(|error| {
                        miette!(error).wrap_err(format!(
                            "cannot create target parent `{}`",
                            current.display()
                        ))
                    })?;
            }
            Err(error) => {
                return Err(miette!(error).wrap_err(format!(
                    "cannot inspect target parent `{}`",
                    current.display()
                )));
            }
        }
    }
    Ok(())
}
