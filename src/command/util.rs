use std::collections::HashMap;
use std::fs::{self, File};
use std::hash::Hasher;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, Result, eyre};
use glob::MatchOptions;
use twox_hash::XxHash64;

use crate::command::apply::PortalEntry;
use crate::config::Config;
use crate::error::IoError;
use crate::{config::DeployType, db::Db};

const SEED: u64 = 42;
const BUFFER_SIZE: usize = 8192;
pub const GLOB_OPTION: MatchOptions = MatchOptions {
    case_sensitive: true,
    require_literal_separator: true,
    require_literal_leading_dot: false,
};

pub fn resolve_target(target_override: Option<PathBuf>, config: &Config) -> Result<PathBuf> {
    let path = &target_override
        .or(config.target_dir.clone())
        .or(dirs::home_dir())
        .ok_or_else(|| eyre!("Cannot determine target directory."))?;

    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Err(eyre!(
            "Target directory must be an absolute path: `{}`.",
            path.display()
        ))
    }
}

pub fn validate_paths(source_dir: &Path, target_dir: &Path) -> Result<()> {
    if source_dir == target_dir {
        return Err(eyre!("Source directory cannot equal target directory."));
    }

    if target_dir.starts_with(source_dir) {
        return Err(eyre!("Target directory cannot be inside source directory."));
    }

    Ok(())
}

pub fn is_glob(pattern: &str) -> bool {
    pattern.contains(['*', '?', '['])
}

pub fn stripping_prefix(glob_pattern: &str) -> String {
    let mut prefix = String::new();
    for component in glob_pattern.split('/') {
        if is_glob(component) {
            break;
        }
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(component);
    }
    if !prefix.is_empty() {
        prefix.push('/');
    }
    prefix
}

pub fn is_actual_dir(path: &Path) -> bool {
    if path.is_symlink() {
        return false;
    }

    path.is_dir()
}

pub fn hash_file(path: &Path) -> Result<u64> {
    let file =
        File::open(path).wrap_err_with(|| format!("Failed to open `{}`.", path.display()))?;
    let mut reader = BufReader::with_capacity(BUFFER_SIZE, file);
    let mut hasher = XxHash64::with_seed(SEED);
    let mut buffer = [0u8; BUFFER_SIZE];

    loop {
        let bytes_read = reader
            .read(&mut buffer)
            .wrap_err_with(|| format!("Failed to read from `{}`.", path.display()))?;
        if bytes_read == 0 {
            break;
        }
        hasher.write(&buffer[..bytes_read]);
    }

    Ok(hasher.finish())
}

pub fn is_managed(target: &Path, db: &Db) -> bool {
    let db_entry = match db.get_entry(target).ok() {
        Some(Some(e)) => e,
        _ => return false,
    };

    match (db_entry.action_type, target.is_symlink()) {
        (DeployType::Symlink, true) | (DeployType::Copy, false) => {}
        _ => return false,
    }

    match db_entry.action_type {
        DeployType::Symlink => match fs::read_link(target) {
            Ok(p) => p == db_entry.reference,
            Err(_) => false,
        },
        DeployType::Copy => match hash_file(target) {
            Ok(h) => Some(h) == db_entry.hash,
            Err(_) => false,
        },
    }
}

pub fn clean_up(
    portal_entries: Option<&HashMap<PathBuf, PortalEntry>>,
    db: &Db,
    dry_run: bool,
    prune_empty_dirs: bool,
) -> Result<()> {
    let db_entries = db.get_all_entries()?;

    for entry in db_entries {
        let path = &entry.target_path;
        if portal_entries.is_some_and(|m| m.contains_key(path)) {
            continue;
        }

        if path.exists() {
            let managed = is_managed(path, db);
            if managed {
                if dry_run {
                    println!("[REMOVE] {}", path.display());
                } else {
                    fs::remove_file(path).remove_file_error(path)?;

                    if prune_empty_dirs {
                        let mut current = path.parent();
                        while let Some(dir) = current {
                            if let Ok(iter) = dir.read_dir()
                                && iter.count() == 0
                            {
                                fs::remove_dir(dir).remove_dir_error(dir)?;
                            } else {
                                break;
                            }
                            current = dir.parent();
                        }
                    }
                }
            }
        }

        if !dry_run {
            db.delete_entry(path)?;
        }
    }

    Ok(())
}

pub fn print_portal(target: &Path, source: &Path, deploy_type: DeployType) -> String {
    format!(
        "{} -> {} ({})",
        target.display(),
        source.display(),
        match deploy_type {
            DeployType::Symlink => "symlink",
            _ => "file",
        },
    )
}

#[cfg(test)]
pub mod tests {
    use super::*;

    use crate::db::DbEntry;
    use std::os::unix::fs as unix_fs;
    use std::path::PathBuf;
    use tempfile::tempdir;
    use test_case::test_case;

    pub fn setup_test(
        portal: &str,
        ignore: &str,
        rule: &str,
        populate: bool,
    ) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp_dir = tempdir().unwrap();
        let source_dir = temp_dir.path().join("source");
        let target_dir = temp_dir.path().join("target");
        fs::create_dir(&source_dir).unwrap();
        fs::create_dir(&target_dir).unwrap();

        if populate {
            fs::write(source_dir.join("a.txt"), "").unwrap();
            fs::write(source_dir.join("b.txt"), "").unwrap();
            fs::create_dir(source_dir.join("subdir")).unwrap();
            fs::write(source_dir.join("subdir").join("c.txt"), "").unwrap();
            fs::write(source_dir.join("subdir").join("d.txt"), "").unwrap();
        }

        let config = format!("ignore = [{ignore}]\n[portal]\n{portal}\n[rule]\n{rule}");
        fs::write(source_dir.join("dotrift.toml"), config).unwrap();

        (temp_dir, source_dir, target_dir)
    }

    #[test_case(|s, t| {
        unix_fs::symlink(s.join("file"), t.join("link")).unwrap();
    },
    |t| t.join("link"),
    |s, t| Some(DbEntry { target_path: t.join("link"), action_type: DeployType::Symlink, reference: s.join("file"), hash: None })
    => true; "symlink_matching_source")]
    #[test_case(|_, t| {
        fs::write(t.join("file"), "").unwrap();
    },
    |t| t.join("file"),
    |s, t| Some(DbEntry { target_path: t.join("file"), action_type: DeployType::Copy, reference: s.join("file"), hash: Some(hash_file(&t.join("file")).unwrap()) })
    => true; "copy_matching_hash")]
    #[test_case(|s, t| {
        unix_fs::symlink(s.join("file1"), t.join("link")).unwrap();
    },
    |t| t.join("link"),
    |s, t| Some(DbEntry { target_path: t.join("link"), action_type: DeployType::Symlink, reference: s.join("file2"), hash: None })
    => false; "symlink_different_source")]
    #[test_case(|s, t| {
        fs::write(s.join("file"), "a").unwrap();
        fs::write(t.join("file"), "b").unwrap();
    },
    |t| t.join("file"),
    |s, t| Some(DbEntry { target_path: t.join("file"), action_type: DeployType::Copy, reference: s.join("file"), hash: Some(hash_file(&s.join("file")).unwrap()) })
    => false; "copy_different_hash")]
    #[test_case(|s, t| {
        fs::write(s.join("file"), "").unwrap();
        unix_fs::symlink(s.join("file"), t.join("link")).unwrap();
    },
    |t| t.join("link"),
    |s, t| Some(DbEntry { target_path: t.join("link"), action_type: DeployType::Copy, reference: s.join("file"), hash: Some(hash_file(&s.join("file")).unwrap()) })
    => false; "symlink_db_is_copy")]
    #[test_case(|_, t| {
        fs::write(t.join("file"), "").unwrap();
    },
    |t| t.join("file"),
    |s, t| Some(DbEntry { target_path: t.join("file"), action_type: DeployType::Symlink, reference: s.join("file"), hash: None })
    => false; "copy_db_is_symlink")]
    #[test_case(|_, t| {
        fs::write(t.join("file"), "").unwrap();
    },
    |t| t.join("file"),
    |_, _| None
    => false; "no_db_entry")]
    fn test_is_managed(
        cb: impl FnOnce(&Path, &Path),
        target_path: impl FnOnce(&Path) -> PathBuf,
        db_entry: impl FnOnce(&Path, &Path) -> Option<DbEntry>,
    ) -> bool {
        let temp_dir = tempdir().unwrap();
        let source_dir = temp_dir.path().join("source");
        let target_dir = temp_dir.path().join("target");
        fs::create_dir_all(&source_dir).unwrap();
        fs::create_dir_all(&target_dir).unwrap();

        cb(&source_dir, &target_dir);

        let db = Db::init(&temp_dir.path().join("db")).unwrap();
        if let Some(e) = db_entry(&source_dir, &target_dir) {
            db.insert_or_update(&e).unwrap();
        }

        is_managed(&target_path(&target_dir), &db)
    }
}
