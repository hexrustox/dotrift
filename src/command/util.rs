use std::{
    collections::HashMap,
    fmt::Debug,
    fs::{self, File},
    hash::Hasher,
    io::{BufReader, Read},
    os::unix::fs as unix_fs,
    path::{Path, PathBuf},
};

use color_eyre::eyre::{Context, Result, eyre};
use glob::MatchOptions;
use normalize_path::NormalizePath;
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

pub fn resolve_target(
    source_dir: &Path,
    target_override: Option<PathBuf>,
    config: &Config,
) -> Result<PathBuf> {
    let target_dir = &target_override
        .or(config.target_dir.clone())
        .or(dirs::home_dir())
        .ok_or_else(|| eyre!("Cannot determine target directory"))?
        .normalize();

    if !target_dir.is_absolute() {
        return Err(eyre!(
            "Target directory must be an absolute path: `{}`",
            target_dir.display()
        ));
    }

    if target_dir.starts_with(source_dir) {
        return Err(eyre!("Target directory cannot be inside source directory"));
    }

    Ok(target_dir.to_path_buf())
}

pub trait SafeStripPrefix<P> {
    fn safe_strip_prefix(&self, base: P) -> &Path;
}

impl<P: AsRef<Path> + Debug> SafeStripPrefix<P> for Path {
    fn safe_strip_prefix(&self, base: P) -> &Path {
        match self.strip_prefix(&base) {
            Ok(p) => p,
            Err(_) => {
                if cfg!(test) {
                    panic!("{base:?} is not prefix of {self:?}");
                } else {
                    self
                }
            }
        }
    }
}

pub fn is_glob(pattern: &str) -> bool {
    pattern.contains(['*', '?', '[', ']'])
}

pub fn strip_prefix_filter_glob(glob_pattern: &str) -> String {
    let mut prefix = String::with_capacity(glob_pattern.len());
    for component in glob_pattern.split('/') {
        if is_glob(component) {
            break;
        }
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(component);
    }
    prefix
}

pub trait PathLiteral {
    fn literal_exists(&self) -> bool;
    fn is_literal_file(&self) -> bool;
    fn is_literal_dir(&self) -> bool;
    fn is_literal_symlink(&self) -> bool;
}

impl PathLiteral for Path {
    fn literal_exists(&self) -> bool {
        fs::symlink_metadata(self).is_ok()
    }
    fn is_literal_file(&self) -> bool {
        fs::symlink_metadata(self).is_ok_and(|m| m.is_file())
    }
    fn is_literal_dir(&self) -> bool {
        fs::symlink_metadata(self).is_ok_and(|m| m.is_dir())
    }
    fn is_literal_symlink(&self) -> bool {
        fs::symlink_metadata(self).is_ok_and(|m| m.is_symlink())
    }
}

pub fn hash_file(path: &Path) -> Result<u64> {
    let file = File::open(path).wrap_err_with(|| format!("Failed to open `{}`", path.display()))?;
    let mut reader = BufReader::with_capacity(BUFFER_SIZE, file);
    let mut hasher = XxHash64::with_seed(SEED);
    let mut buffer = [0u8; BUFFER_SIZE];

    loop {
        let bytes_read = reader
            .read(&mut buffer)
            .wrap_err_with(|| format!("Failed to read from `{}`", path.display()))?;
        if bytes_read == 0 {
            break;
        }
        hasher.write(&buffer[..bytes_read]);
    }

    Ok(hasher.finish())
}

pub fn is_managed(target: &Path, db: &Db, target_hash: Option<u64>) -> bool {
    let db_entry = match db.get_entry(target).ok() {
        Some(Some(e)) => e,
        _ => return false,
    };

    match db_entry.deploy_type {
        DeployType::Symlink => match fs::read_link(target) {
            Ok(p) => p == db_entry.source_path,
            Err(_) => false,
        },
        DeployType::Copy => {
            if let Some(hash) = db_entry.hash {
                target.is_literal_file()
                    && hash
                        == target_hash.unwrap_or({
                            let Ok(h) = hash_file(target) else {
                                return false;
                            };
                            h
                        })
            } else {
                target.is_literal_symlink()
                    && fs::read_link(target).is_ok_and(|l| Some(l) == db_entry.symlink_target)
            }
        }
    }
}

pub fn copy_recursive(from: &Path, to: &Path) -> Result<()> {
    if from.is_literal_dir() {
        fs::create_dir_all(to).create_dir_error(to)?;
        for entry in fs::read_dir(from)
            .wrap_err_with(|| format!("Failed to read `{}`", from.display()))?
            .flatten()
        {
            let path = entry.path();
            let suffix = path.safe_strip_prefix(from);
            copy_recursive(&path, &to.join(suffix))?;
        }
    } else {
        clone_file(from, to)?;
    }

    Ok(())
}

pub fn clone_file(from: &Path, to: &Path) -> Result<()> {
    if from.is_literal_file() {
        fs::copy(from, to).copy_file_error(from, to)?;
    } else if from.is_literal_symlink() {
        let _ = fs::remove_file(to);
        unix_fs::symlink(fs::read_link(from).read_link_error(from)?, to).symlink_error(to)?;
    } else {
        #[cfg(test)]
        panic!("{:?} is not a directory", from);
    }

    Ok(())
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

        if path.literal_exists() {
            let managed = is_managed(path, db, None);
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

    // -- Symlink deploy type --
    #[test_case(
        |s, t| { unix_fs::symlink(s.join("file"), t.join("link")).unwrap(); },
        |t| t.join("link"),
        |s, t| Some(DbEntry {
            target_path: t.join("link"),
            deploy_type: DeployType::Symlink,
            source_path: s.join("file"),
            hash: None,
            symlink_target: None,
        })
        => true; "symlink_matching_source")]
    #[test_case(
        |s, t| { unix_fs::symlink(s.join("file1"), t.join("link")).unwrap(); },
        |t| t.join("link"),
        |s, t| Some(DbEntry {
            target_path: t.join("link"),
            deploy_type: DeployType::Symlink,
            source_path: s.join("file2"),
            hash: None,
            symlink_target: None,
        })
        => false; "symlink_different_source")]
    #[test_case(
        |_, _| {},
        |t| t.join("missing"),
        |s, t| Some(DbEntry {
            target_path: t.join("missing"),
            deploy_type: DeployType::Symlink,
            source_path: s.join("link"),
            hash: None,
            symlink_target: None,
        })
        => false; "symlink_target_missing")]
    // -- Copy deploy type with hash --
    #[test_case(
        |_, t| { fs::write(t.join("file"), "").unwrap(); },
        |t| t.join("file"),
        |s, t| Some(DbEntry {
            target_path: t.join("file"),
            deploy_type: DeployType::Copy,
            source_path: s.join("file"),
            hash: Some(hash_file(&t.join("file")).unwrap()),
            symlink_target: None,
        })
        => true; "copy_matching_hash")]
    #[test_case(
        |s, t| {
            fs::write(s.join("file"), "a").unwrap();
            fs::write(t.join("file"), "b").unwrap();
        },
        |t| t.join("file"),
        |s, t| Some(DbEntry {
            target_path: t.join("file"),
            deploy_type: DeployType::Copy,
            source_path: s.join("file"),
            hash: Some(hash_file(&s.join("file")).unwrap()),
            symlink_target: None,
        })
        => false; "copy_different_hash")]
    #[test_case(
        |s, t| {
            fs::write(s.join("file"), "a").unwrap();
            unix_fs::symlink(s.join("file"), t.join("link")).unwrap();
        },
        |t| t.join("link"),
        |s, t| Some(DbEntry {
            target_path: t.join("link"),
            deploy_type: DeployType::Copy,
            source_path: s.join("file"),
            hash: Some(hash_file(&s.join("file")).unwrap()),
            symlink_target: None,
        })
        => false; "copy_hash_target_is_symlink")]
    #[test_case(
        |_, _| {},
        |t| t.join("missing"),
        |_, t| Some(DbEntry {
            target_path: t.join("missing"),
            deploy_type: DeployType::Copy,
            source_path: PathBuf::from("/x"),
            hash: Some(0),
            symlink_target: None,
        })
        => false; "copy_hash_target_missing")]
    // -- Copy deploy type with symlink_target --
    #[test_case(
        |_, t| { unix_fs::symlink(Path::new("/a"), t.join("link")).unwrap(); },
        |t| t.join("link"),
        |_, t| Some(DbEntry {
            target_path: t.join("link"),
            deploy_type: DeployType::Copy,
            source_path: PathBuf::from("/x"),
            hash: None,
            symlink_target: Some(PathBuf::from("/a")),
        })
        => true; "copy_symlink_target_managed")]
    #[test_case(
        |_, t| { unix_fs::symlink(Path::new("/b"), t.join("link")).unwrap(); },
        |t| t.join("link"),
        |_, t| Some(DbEntry {
            target_path: t.join("link"),
            deploy_type: DeployType::Copy,
            source_path: PathBuf::from("/x"),
            hash: None,
            symlink_target: Some(PathBuf::from("/a")),
        })
        => false; "copy_symlink_target_mismatch")]
    #[test_case(
        |_, t| { fs::write(t.join("file"), "").unwrap(); },
        |t| t.join("file"),
        |_, t| Some(DbEntry {
            target_path: t.join("file"),
            deploy_type: DeployType::Copy,
            source_path: PathBuf::from("/x"),
            hash: None,
            symlink_target: Some(PathBuf::from("/a")),
        })
        => false; "copy_symlink_target_is_file")]
    #[test_case(
        |_, _| {},
        |t| t.join("missing"),
        |_, t| Some(DbEntry {
            target_path: t.join("missing"),
            deploy_type: DeployType::Copy,
            source_path: PathBuf::from("/x"),
            hash: None,
            symlink_target: Some(PathBuf::from("/a")),
        })
        => false; "copy_symlink_target_missing")]
    // -- Cross-type and corrupt --
    #[test_case(
        |s, t| {
            fs::write(s.join("file"), "").unwrap();
            unix_fs::symlink(s.join("file"), t.join("link")).unwrap();
        },
        |t| t.join("link"),
        |s, t| Some(DbEntry {
            target_path: t.join("link"),
            deploy_type: DeployType::Copy,
            source_path: s.join("file"),
            hash: Some(hash_file(&s.join("file")).unwrap()),
            symlink_target: None,
        })
        => false; "symlink_db_is_copy")]
    #[test_case(
        |_, t| { fs::write(t.join("file"), "").unwrap(); },
        |t| t.join("file"),
        |s, t| Some(DbEntry {
            target_path: t.join("file"),
            deploy_type: DeployType::Symlink,
            source_path: s.join("file"),
            hash: None,
            symlink_target: None,
        })
        => false; "copy_db_is_symlink")]
    #[test_case(
        |_, t| { fs::write(t.join("file"), "").unwrap(); },
        |t| t.join("file"),
        |_, _| None
        => false; "no_db_entry")]
    #[test_case(
        |_, t| { fs::write(t.join("file"), "").unwrap(); },
        |t| t.join("file"),
        |_, t| Some(DbEntry {
            target_path: t.join("file"),
            deploy_type: DeployType::Copy,
            source_path: PathBuf::from("/x"),
            hash: None,
            symlink_target: None,
        })
        => false; "copy_corrupt_no_hash_no_symlink_target")]
    fn test_is_managed(
        cb: impl FnOnce(&Path, &Path),
        target_path: impl FnOnce(&Path) -> PathBuf,
        db_entry: impl FnOnce(&Path, &Path) -> Option<DbEntry>,
    ) -> bool {
        let temp_dir = tempdir().unwrap();
        let source_dir = temp_dir.path().join("source");
        let target_dir = temp_dir.path().join("target");
        fs::create_dir(&source_dir).unwrap();
        fs::create_dir(&target_dir).unwrap();

        cb(&source_dir, &target_dir);

        let db = Db::init(&temp_dir.path().join("db")).unwrap();
        if let Some(e) = db_entry(&source_dir, &target_dir) {
            db.insert_or_update(&e).unwrap();
        }

        is_managed(&target_path(&target_dir), &db, None)
    }

    #[test_case(|t| {
        fs::write(t.join("file1"), "a").unwrap();
        (t.join("file1"), t.join("file2"))
    }, |t| {
        assert_eq!(fs::read_to_string(t).unwrap(), "a");
    }; "file")]
    #[test_case(|t| {
        unix_fs::symlink(Path::new("/a"), t.join("file1")).unwrap();
        (t.join("file1"), t.join("file2"))
    }, |t| {
        assert_eq!(fs::read_link(t).unwrap(), Path::new("/a"));
    }; "symlink")]
    #[test_case(|t| {
        fs::create_dir_all(t.join("dir1/subdir")).unwrap();
        fs::write(t.join("dir1/file1"), "").unwrap();
        fs::write(t.join("dir1/file2"), "").unwrap();
        fs::write(t.join("dir1/subdir/file3"), "").unwrap();
        (t.join("dir1"), t.join("dir2"))
    }, |t| {
        assert!(t.join("file1").exists());
        assert!(t.join("file2").exists());
        assert!(t.join("subdir/file3").exists());
    }; "directory")]
    fn test_copy_recursive(
        setup: impl FnOnce(&Path) -> (PathBuf, PathBuf),
        assertion: impl FnOnce(&Path),
    ) {
        let temp_dir = tempdir().unwrap();
        let (f, t) = setup(temp_dir.path());
        copy_recursive(&f, &t).unwrap();
        assertion(&t);
    }
}
