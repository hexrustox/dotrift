use std::{
    collections::HashMap,
    env,
    fmt::Debug,
    fs::{self, File},
    hash::Hasher,
    io::{BufReader, ErrorKind, Read},
    os::unix::fs as unix_fs,
    path::{Path, PathBuf},
};

use glob::{MatchOptions, glob_with};
use ignore::gitignore::Gitignore;
use miette::{Context, Result, miette};
use normalize_path::NormalizePath;
use twox_hash::XxHash64;
use walkdir::WalkDir;

use crate::{
    command::apply::PortalEntry,
    config::{Config, DeployType},
    db::{Db, DbEntry},
    output,
};

const SEED: u64 = 42;
const BUFFER_SIZE: usize = 8192;

pub const GLOB_OPTION: MatchOptions = MatchOptions {
    case_sensitive: true,
    require_literal_separator: true,
    require_literal_leading_dot: false,
};

pub fn to_absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.normalize())
    } else {
        let cwd = env::current_dir()
            .map_err(|e| miette!(e))
            .wrap_err("failed to get current directory")?;
        Ok(cwd.join(path).normalize())
    }
}

pub fn resolve_target(
    source_dir: &Path,
    target_override: Option<PathBuf>,
    config: &Config,
) -> Result<PathBuf> {
    let target_dir = &target_override
        .or(config.target_dir.clone())
        .or(dirs::home_dir())
        .ok_or_else(|| {
            miette!(
                help = "provide --target flag, set target-directory in config, or set $HOME",
                "cannot determine target directory"
            )
        })?
        .normalize();

    if !target_dir.is_absolute() {
        return Err(miette!(
            "target directory must be an absolute path: `{}`",
            target_dir.display()
        ));
    }

    if target_dir.starts_with(source_dir) {
        return Err(miette!(
            "target directory `{}` cannot be inside source directory `{}`",
            target_dir.display(),
            source_dir.display()
        ));
    }

    Ok(target_dir.to_path_buf())
}

pub trait StripPrefixOrSelf<P> {
    fn safe_strip_prefix(&self, base: P) -> &Path;
}

impl<P: AsRef<Path> + Debug> StripPrefixOrSelf<P> for Path {
    fn safe_strip_prefix(&self, base: P) -> &Path {
        match self.strip_prefix(&base) {
            Ok(p) => p,
            Err(_) => {
                #[cfg(test)]
                panic!("{base:?} is not prefix of {self:?}");
                #[cfg(not(test))]
                self
            }
        }
    }
}

pub fn is_glob(pattern: &str) -> bool {
    pattern.contains(['*', '?', '[', ']'])
}

fn strip_prefix_filter_glob(glob_pattern: &str) -> String {
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

pub trait PathExt {
    fn path_exists(&self) -> bool;
    fn path_is_file(&self) -> bool;
    fn path_is_dir(&self) -> bool;
    fn path_is_symlink(&self) -> bool;
}

impl PathExt for Path {
    fn path_exists(&self) -> bool {
        fs::symlink_metadata(self).is_ok()
    }
    fn path_is_file(&self) -> bool {
        fs::symlink_metadata(self).is_ok_and(|m| m.is_file())
    }
    fn path_is_dir(&self) -> bool {
        fs::symlink_metadata(self).is_ok_and(|m| m.is_dir())
    }
    fn path_is_symlink(&self) -> bool {
        fs::symlink_metadata(self).is_ok_and(|m| m.is_symlink())
    }
}

pub fn resolve_portal_entries<F>(
    source_dir: &Path,
    target_dir: &Path,
    portals: &HashMap<String, PathBuf>,
    ignore_matcher: &Gitignore,
    skip_missing: bool,
    mut on_entry: F,
) -> Result<()>
where
    F: FnMut(PathBuf, PathBuf, String) -> Result<()>,
{
    for (pattern, target_rel) in portals {
        let pattern_normalized = Path::new(pattern).normalize();
        let target_rel_normalized = target_rel.normalize();
        let pattern_key = pattern_normalized.to_string_lossy().into_owned();

        if is_glob(&pattern_key) {
            let prefix = strip_prefix_filter_glob(&pattern_key);
            let full_pattern = source_dir.join(&pattern_normalized);
            let full_pattern_str = full_pattern.to_string_lossy();

            let paths = match crate::glob_err!(
                glob_with(&full_pattern_str, GLOB_OPTION),
                &full_pattern_str
            ) {
                Ok(p) => p,
                Err(e) => {
                    if skip_missing {
                        continue;
                    }
                    return Err(e);
                }
            };

            for source_path in paths.flatten() {
                if source_path.path_is_dir() {
                    continue;
                }

                let source_rel = source_path.safe_strip_prefix(source_dir);

                let stripped = if prefix.is_empty() {
                    source_rel.to_path_buf()
                } else {
                    source_rel
                        .safe_strip_prefix(Path::new(&prefix))
                        .to_path_buf()
                };

                let target_path = target_dir.join(&target_rel_normalized).join(stripped);

                if is_ignored(ignore_matcher, &target_path) {
                    continue;
                }

                on_entry(source_path, target_path, pattern_key.clone())?;
            }
        } else {
            let source_path = source_dir.join(&pattern_normalized);

            if !source_path.path_exists() {
                if skip_missing {
                    continue;
                }
                return Err(miette!(
                    "source path does not exist: `{}`",
                    source_path.display()
                ));
            }

            if source_path.path_is_dir() {
                for entry in walk_files(&source_path) {
                    let file_source = entry.path().to_path_buf();

                    let rel_to_pattern = file_source.safe_strip_prefix(&source_path);

                    let target_path = target_dir.join(&target_rel_normalized).join(rel_to_pattern);

                    if is_ignored(ignore_matcher, &target_path) {
                        continue;
                    }

                    on_entry(file_source, target_path, pattern_key.clone())?;
                }
            } else {
                let target_path = target_dir.join(&target_rel_normalized);

                if is_ignored(ignore_matcher, &target_path) {
                    continue;
                }

                on_entry(source_path, target_path, pattern_key)?;
            }
        }
    }

    Ok(())
}

fn is_ignored(matcher: &Gitignore, path: &Path) -> bool {
    matcher.matched_path_or_any_parents(path, false).is_ignore()
}

fn walk_all(path: &Path) -> impl Iterator<Item = walkdir::DirEntry> + '_ {
    WalkDir::new(path).into_iter().filter_map(|e| match e {
        Ok(entry) => Some(entry),
        Err(err) => {
            if let Some(path) = err.path() {
                output::print_warn(format!("inaccessible `{}`, skipping", path.display()));
            } else {
                output::print_warn(format!("{}, skipping", err));
            }
            None
        }
    })
}

pub fn walk_files(path: &Path) -> impl Iterator<Item = walkdir::DirEntry> + '_ {
    walk_all(path).filter(|e| !e.file_type().is_dir())
}

pub fn read_mtime(path: &Path) -> Option<i64> {
    let time = path.symlink_metadata().ok()?.modified().ok()?;
    crate::time::epoch_ms(Some(time)).ok()
}

pub fn hash_file(path: &Path) -> Result<u64> {
    let file = File::open(path)
        .map_err(|e| miette!(e))
        .wrap_err_with(|| format!("failed to open `{}`", path.display()))?;
    let mut reader = BufReader::with_capacity(BUFFER_SIZE, file);
    let mut hasher = XxHash64::with_seed(SEED);
    let mut buffer = [0u8; BUFFER_SIZE];

    loop {
        let bytes_read = reader
            .read(&mut buffer)
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("failed to read from `{}`", path.display()))?;
        if bytes_read == 0 {
            break;
        }
        hasher.write(&buffer[..bytes_read]);
    }

    Ok(hasher.finish())
}

pub fn is_managed_entry(entry: &DbEntry, target: &Path, target_hash: Option<u64>) -> bool {
    match entry.deploy_type {
        DeployType::Symlink => match fs::read_link(target) {
            Ok(p) => p == entry.source_path,
            Err(_) => false,
        },
        DeployType::Copy | DeployType::Tmpl => {
            if let Some(hash) = entry.hash {
                target.path_is_file() && {
                    if entry
                        .mtime
                        .is_some_and(|mt| read_mtime(target).is_some_and(|t| t == mt))
                    {
                        true
                    } else if let Ok(h) = target_hash.map(Ok).unwrap_or_else(|| hash_file(target)) {
                        h == hash
                    } else {
                        false
                    }
                }
            } else {
                target.path_is_symlink()
                    && fs::read_link(target).is_ok_and(|l| Some(l) == entry.symlink_target)
            }
        }
    }
}

pub fn is_managed(target: &Path, db: &Db, target_hash: Option<u64>) -> bool {
    match db.get_entry(target).ok().flatten() {
        Some(entry) => is_managed_entry(&entry, target, target_hash),
        None => false,
    }
}

pub fn copy_recursive(from: &Path, to: &Path) -> Result<()> {
    if from.path_is_dir() {
        for entry in walk_all(from) {
            let path = entry.path();
            let base = path.safe_strip_prefix(from);
            let new = to.join(base);
            if path.path_is_dir() {
                if !new.path_exists() {
                    crate::create_dir_err!(fs::create_dir(&new), &new)?;
                }
            } else {
                clone_file(path, &new)?;
            }
        }
    } else {
        clone_file(from, to)?;
    }
    Ok(())
}

pub fn clone_file(from: &Path, to: &Path) -> Result<()> {
    let _ = fs::remove_file(to);
    if from.path_is_file() {
        crate::copy_file_err!(fs::copy(from, to), from, to)?;
    } else if from.path_is_symlink() {
        crate::symlink_err!(
            unix_fs::symlink(crate::read_link_err!(fs::read_link(from), from)?, to),
            to,
            from
        )?;
    } else {
        return Err(miette!(
            "cannot clone `{}`: unsupported file type",
            from.display()
        ));
    }

    Ok(())
}

pub fn clean_up(
    portal_entries: &HashMap<PathBuf, PortalEntry>,
    db: &Db,
    dry_run: bool,
    prune_empty_dirs: bool,
    verbose: bool,
    invert: bool,
) -> Result<usize> {
    let db_entries = db.get_all_entries()?;

    let mut count = 0;
    for entry in db_entries {
        let path = &entry.target_path;
        let in_portal = portal_entries.contains_key(path);
        if invert {
            if !in_portal {
                continue;
            }
        } else {
            if in_portal {
                continue;
            }
        }

        if path.path_exists() {
            let managed = is_managed_entry(&entry, path, None);
            if managed {
                if dry_run {
                    count += 1;
                    output::print_dry_remove(path);
                } else {
                    crate::remove_file_err!(fs::remove_file(path), path)?;
                    if verbose {
                        output::print_removed(path);
                    }

                    if prune_empty_dirs {
                        for dir in path.ancestors().skip(1) {
                            match fs::remove_dir(dir) {
                                Ok(()) => {}
                                Err(e) if e.kind() == ErrorKind::NotFound => {}
                                Err(e) if e.kind() == ErrorKind::DirectoryNotEmpty => break,
                                Err(e) => {
                                    crate::remove_dir_err!(Err::<(), _>(e), dir)?;
                                }
                            }
                        }
                    }
                }
            }
        }

        if !dry_run {
            db.delete_entry(path)?;
        }
    }

    Ok(count)
}

#[cfg(test)]
pub fn assert_captured_output(label: &str, temp_path: &Path) {
    use insta::{assert_snapshot, with_settings};
    use strip_ansi_escapes::strip_str;

    let mut paths = Vec::new();
    let mut current = temp_path;
    loop {
        if current.components().count() > 1 {
            paths.push(current.to_string_lossy().to_string());
        }
        if let Some(p) = current.parent() {
            current = p;
        } else {
            break;
        }
    }
    let filters = paths.iter().map(|p| (p.as_str(), "@"));
    with_settings!({filters => filters}, {
        assert_snapshot!(label, strip_str(crate::output::test_capture::take_all()));
    });
}

#[cfg(test)]
pub mod tests {
    use super::*;

    use crate::db::DbEntry;
    use std::os::unix::fs::{self as unix_fs, PermissionsExt};
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
            fs::write(source_dir.join("file.txt"), "").unwrap();
            unix_fs::symlink(source_dir.join("file.txt"), source_dir.join("link.txt")).unwrap();
            unix_fs::symlink(
                Path::new("/nonexistent/dotrift_test_target"),
                source_dir.join("broken.txt"),
            )
            .unwrap();
            fs::create_dir(source_dir.join("subdir")).unwrap();
            fs::create_dir(source_dir.join("subdir").join("nested")).unwrap();
            fs::write(
                source_dir.join("subdir").join("nested").join("inner.txt"),
                "",
            )
            .unwrap();
            unix_fs::symlink(source_dir.join("subdir"), source_dir.join("linkdir")).unwrap();
        }

        let config = format!("ignore = [{ignore}]\n[portal]\n{portal}\n[rule]\n{rule}");
        fs::write(source_dir.join("dotrift.toml"), config).unwrap();

        (temp_dir, source_dir, target_dir)
    }

    #[test_case(
        |s, t| {
            unix_fs::symlink(s.join("file"), t.join("link")).unwrap();
        },
        |t| t.join("link"),
        |s, t| Some(DbEntry {
            target_path: t.join("link"),
            deploy_type: DeployType::Symlink,
            source_path: s.join("file"),
            hash: None,
            symlink_target: None,
            mtime: None,
        })
        => true; "symlink_matching_source"
    )]
    #[test_case(
        |s, t| {
            unix_fs::symlink(s.join("file1"), t.join("link")).unwrap();
        },
        |t| t.join("link"),
        |s, t| Some(DbEntry {
            target_path: t.join("link"),
            deploy_type: DeployType::Symlink,
            source_path: s.join("file2"),
            hash: None,
            symlink_target: None,
            mtime: None,
        })
        => false; "symlink_different_source"
    )]
    #[test_case(
        |_, _| {},
        |t| t.join("missing"),
        |s, t| Some(DbEntry {
            target_path: t.join("missing"),
            deploy_type: DeployType::Symlink,
            source_path: s.join("link"),
            hash: None,
            symlink_target: None,
            mtime: None,
        })
        => false; "symlink_target_missing"
    )]
    #[test_case(
        |_, t| {
            fs::write(t.join("file"), "").unwrap();
        },
        |t| t.join("file"),
        |s, t| Some(DbEntry {
            target_path: t.join("file"),
            deploy_type: DeployType::Copy,
            source_path: s.join("file"),
            hash: Some(hash_file(&t.join("file")).unwrap()),
            symlink_target: None,
            mtime: None,
        })
        => true; "copy_matching_hash"
    )]
    #[test_case(
        |_, t| {
            fs::write(t.join("file"), "same").unwrap();
        },
        |t| t.join("file"),
        |_, t| {
            Some(DbEntry {
                target_path: t.join("file"),
                deploy_type: DeployType::Copy,
                source_path: PathBuf::from("/x"),
                hash: Some(0),
                symlink_target: None,
                mtime: read_mtime(&t.join("file")),
            })
        }
        => true; "copy_mtime_match"
    )]
    #[test_case(
        |_, t| {
            fs::write(t.join("file"), "same").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(200));
            fs::write(t.join("file"), "same").unwrap();
        },
        |t| t.join("file"),
        |_, t| Some(DbEntry {
            target_path: t.join("file"),
            deploy_type: DeployType::Copy,
            source_path: PathBuf::from("/x"),
            hash: Some(hash_file(&t.join("file")).unwrap()),
            symlink_target: None,
            mtime: Some(0),
        })
        => true; "copy_mtime_mismatch_hash_match"
    )]
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
            mtime: None,
        })
        => false; "copy_different_hash"
    )]
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
            mtime: None,
        })
        => false; "copy_hash_target_is_symlink"
    )]
    #[test_case(
        |_, _| {},
        |t| t.join("missing"),
        |_, t| Some(DbEntry {
            target_path: t.join("missing"),
            deploy_type: DeployType::Copy,
            source_path: PathBuf::from("/x"),
            hash: Some(0),
            symlink_target: None,
            mtime: None,
        })
        => false; "copy_hash_target_missing"
    )]
    #[test_case(
        |_, t| {
            unix_fs::symlink(Path::new("/a"), t.join("link")).unwrap();
        },
        |t| t.join("link"),
        |_, t| Some(DbEntry {
            target_path: t.join("link"),
            deploy_type: DeployType::Copy,
            source_path: PathBuf::from("/x"),
            hash: None,
            symlink_target: Some(PathBuf::from("/a")),
            mtime: None,
        })
        => true; "copy_symlink_target_managed"
    )]
    #[test_case(
        |_, t| {
            unix_fs::symlink(Path::new("/b"), t.join("link")).unwrap();
        },
        |t| t.join("link"),
        |_, t| Some(DbEntry {
            target_path: t.join("link"),
            deploy_type: DeployType::Copy,
            source_path: PathBuf::from("/x"),
            hash: None,
            symlink_target: Some(PathBuf::from("/a")),
            mtime: None,
        })
        => false; "copy_symlink_target_mismatch"
    )]
    #[test_case(
        |_, t| {
            fs::write(t.join("file"), "").unwrap();
        },
        |t| t.join("file"),
        |_, t| Some(DbEntry {
            target_path: t.join("file"),
            deploy_type: DeployType::Copy,
            source_path: PathBuf::from("/x"),
            hash: None,
            symlink_target: Some(PathBuf::from("/a")),
            mtime: None,
        })
        => false; "copy_symlink_target_is_file"
    )]
    #[test_case(
        |_, _| {},
        |t| t.join("missing"),
        |_, t| Some(DbEntry {
            target_path: t.join("missing"),
            deploy_type: DeployType::Copy,
            source_path: PathBuf::from("/x"),
            hash: None,
            symlink_target: Some(PathBuf::from("/a")),
            mtime: None,
        })
        => false; "copy_symlink_target_missing"
    )]
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
            mtime: None,
        })
        => false; "symlink_db_is_copy"
    )]
    #[test_case(
        |_, t| {
            fs::write(t.join("file"), "").unwrap();
        },
        |t| t.join("file"),
        |s, t| Some(DbEntry {
            target_path: t.join("file"),
            deploy_type: DeployType::Symlink,
            source_path: s.join("file"),
            hash: None,
            symlink_target: None,
            mtime: None,
        })
        => false; "copy_db_is_symlink"
    )]
    #[test_case(
        |_, t| {
            fs::write(t.join("file"), "").unwrap();
        },
        |t| t.join("file"),
        |_, _| None
        => false; "no_db_entry"
    )]
    #[test_case(
        |_, t| {
            fs::write(t.join("file"), "").unwrap();
        },
        |t| t.join("file"),
        |_, t| Some(DbEntry {
            target_path: t.join("file"),
            deploy_type: DeployType::Copy,
            source_path: PathBuf::from("/x"),
            hash: None,
            symlink_target: None,
            mtime: None,
        })
        => false; "copy_corrupt_no_hash_no_symlink_target"
    )]
    #[test_case(
        |_, t| {
            fs::create_dir(t.join("dir")).unwrap();
        },
        |t| t.join("dir"),
        |_, t| Some(DbEntry {
            target_path: t.join("dir"),
            deploy_type: DeployType::Copy,
            source_path: PathBuf::from("/x"),
            hash: Some(0),
            symlink_target: None,
            mtime: None,
        })
        => false; "copy_target_is_dir"
    )]
    #[test_case(
        |_, t| {
            fs::write(t.join("file"), "data").unwrap();
            let mut perms = fs::metadata(t.join("file")).unwrap().permissions();
            perms.set_mode(0o000);
            fs::set_permissions(t.join("file"), perms).unwrap();
        },
        |t| t.join("file"),
        |_, t| Some(DbEntry {
            target_path: t.join("file"),
            deploy_type: DeployType::Copy,
            source_path: PathBuf::from("/x"),
            hash: Some(0),
            symlink_target: None,
            mtime: None,
        }) => false; "copy_source_unreadable"
    )]
    #[test_case(
        |_, t| {
            fs::write(t.join("file"), "data").unwrap();
        },
        |t| t.join("file"),
        |_, t| Some(DbEntry {
            target_path: t.join("file"),
            deploy_type: DeployType::Tmpl,
            source_path: PathBuf::from("/x"),
            hash: Some(hash_file(&t.join("file")).unwrap()),
            symlink_target: None,
            mtime: None,
        })
        => true; "tmpl_matching_hash"
    )]
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
    }; "copy_regular_file")]
    #[test_case(|t| {
        fs::write(t.join("file1"), "new").unwrap();
        fs::write(t.join("file2"), "old").unwrap();
        (t.join("file1"), t.join("file2"))
    }, |t| {
        assert_eq!(fs::read_to_string(t).unwrap(), "new");
    }; "copy_regular_file_overwrite")]
    #[test_case(|t| {
        fs::write(t.join("file1"), "data").unwrap();
        unix_fs::symlink(Path::new("/x"), t.join("link")).unwrap();
        (t.join("file1"), t.join("link"))
    }, |t| {
        assert!(t.is_file());
        assert_eq!(fs::read_to_string(t).unwrap(), "data");
    }; "copy_regular_file_overwrite_symlink")]
    #[test_case(|t| {
        unix_fs::symlink(Path::new("/a"), t.join("file1")).unwrap();
        (t.join("file1"), t.join("file2"))
    }, |t| {
        assert_eq!(fs::read_link(t).unwrap(), Path::new("/a"));
    }; "copy_symlink")]
    #[test_case(|t| {
        unix_fs::symlink(Path::new("/a"), t.join("link")).unwrap();
        fs::write(t.join("file2"), "data").unwrap();
        (t.join("link"), t.join("file2"))
    }, |t| {
        assert!(t.is_symlink());
        assert_eq!(fs::read_link(t).unwrap(), Path::new("/a"));
    }; "copy_symlink_overwrite_file")]
    #[test_case(|t| {
        unix_fs::symlink(Path::new("/a"), t.join("link1")).unwrap();
        unix_fs::symlink(Path::new("/b"), t.join("link2")).unwrap();
        (t.join("link1"), t.join("link2"))
    }, |t| {
        assert_eq!(fs::read_link(t).unwrap(), Path::new("/a"));
    }; "copy_symlink_overwrite_symlink")]
    #[test_case(|t| {
        unix_fs::symlink(Path::new("/nonexistent"), t.join("broken")).unwrap();
        (t.join("broken"), t.join("dest"))
    }, |t| {
        assert!(t.is_symlink());
        assert_eq!(fs::read_link(t).unwrap(), Path::new("/nonexistent"));
    }; "copy_broken_symlink")]
    #[test_case(|t| {
        fs::create_dir(t.join("sub")).unwrap();
        unix_fs::symlink(Path::new("../other"), t.join("sub/rel")).unwrap();
        (t.join("sub/rel"), t.join("dest"))
    }, |t| {
        assert!(t.is_symlink());
        assert_eq!(fs::read_link(t).unwrap(), Path::new("../other"));
    }; "copy_relative_symlink")]
    #[test_case(|t| {
        fs::write(t.join("empty"), "").unwrap();
        (t.join("empty"), t.join("dest"))
    }, |t| {
        assert!(t.is_file());
        assert_eq!(fs::read_to_string(t).unwrap(), "");
    }; "copy_empty_file")]
    #[test_case(|t| {
        (t.join("ghost"), t.join("dest"))
    }, |_| {} => panics ""; "source_not_exists")]
    #[test_case(|t| {
        fs::create_dir(t.join("dir")).unwrap();
        fs::write(t.join("file"), "a").unwrap();
        (t.join("file"), t.join("dir"))
    }, |_| {} => panics ""; "target_is_directory")]
    fn test_clone_file(
        setup: impl FnOnce(&Path) -> (PathBuf, PathBuf),
        assertion: impl FnOnce(&Path),
    ) {
        let temp_dir = tempdir().unwrap();
        let (f, t) = setup(temp_dir.path());
        clone_file(&f, &t).unwrap();
        assertion(&t);
    }

    #[test_case(|t| {
        fs::create_dir_all(t.join("dir1/subdir")).unwrap();
        fs::write(t.join("dir1/file1"), "").unwrap();
        fs::write(t.join("dir1/file2"), "").unwrap();
        fs::write(t.join("dir1/subdir/file3"), "").unwrap();
        unix_fs::symlink(Path::new("/a"), t.join("dir1/subdir/file4")).unwrap();
        (t.join("dir1"), t.join("dir2"))
    }, |t| {
        assert!(t.join("file1").exists());
        assert!(t.join("file2").exists());
        assert!(t.join("subdir/file3").exists());
        assert!(t.join("subdir/file4").is_symlink());
    }; "recursive")]
    #[test_case(|t| {
        fs::create_dir_all(t.join("dir1/subdir")).unwrap();
        fs::create_dir_all(t.join("dir2/subdir")).unwrap();
        fs::write(t.join("dir1/subdir/file1"), "").unwrap();
        (t.join("dir1"), t.join("dir2"))
    }, |t| {
        assert!(t.join("subdir/file1").exists());
    }; "recursive_merge_into")]
    #[test_case(|t| {
        fs::create_dir(t.join("empty")).unwrap();
        (t.join("empty"), t.join("dest"))
    }, |t| {
        assert!(t.is_dir());
        assert_eq!(t.read_dir().unwrap().count(), 0);
    }; "recursive_empty_dir")]
    #[test_case(|t| {
        fs::create_dir_all(t.join("src/sub")).unwrap();
        fs::create_dir_all(t.join("dst/sub")).unwrap();
        fs::write(t.join("src/sub/file"), "new").unwrap();
        fs::write(t.join("dst/sub/file"), "old").unwrap();
        (t.join("src"), t.join("dst"))
    }, |t| {
        assert_eq!(fs::read_to_string(t.join("sub/file")).unwrap(), "new");
    }; "recursive_overwrite")]
    #[test_case(|t| {
        fs::create_dir_all(t.join("deep/a/b/c")).unwrap();
        fs::write(t.join("deep/a/b/c/f"), "x").unwrap();
        (t.join("deep"), t.join("dest"))
    }, |t| {
        assert!(t.join("a/b/c/f").exists());
        assert_eq!(fs::read_to_string(t.join("a/b/c/f")).unwrap(), "x");
    }; "recursive_deeply_nested")]
    #[test_case(|t| {
        fs::create_dir_all(t.join("src/b")).unwrap();
        fs::write(t.join("src/key name.txt"), "utf8").unwrap();
        fs::write(t.join("src/b/spaces  here.log"), "x").unwrap();
        (t.join("src"), t.join("dest"))
    }, |t| {
        assert!(t.join("key name.txt").exists());
        assert!(t.join("b/spaces  here.log").exists());
    }; "recursive_special_filenames")]
    fn test_copy_recursive(
        setup: impl FnOnce(&Path) -> (PathBuf, PathBuf),
        assertion: impl FnOnce(&Path),
    ) {
        let temp_dir = tempdir().unwrap();
        let (f, t) = setup(temp_dir.path());
        copy_recursive(&f, &t).unwrap();
        assertion(&t);
    }

    #[test_case(
        |target, db| {
            let source = target.parent().unwrap().join("source");
            fs::create_dir_all(&source).unwrap();
            unix_fs::symlink(source.join("file"), target.join("link")).unwrap();
            db.insert_or_update(&DbEntry {
                target_path: target.join("link"),
                deploy_type: DeployType::Symlink,
                source_path: source.join("file"),
                hash: None,
                symlink_target: None,
                mtime: None,
            }).unwrap();
        },
        false, false, false,
        |target, db| {
            assert!(!target.join("link").is_symlink());
            assert!(db.get_entry(&target.join("link")).unwrap().is_none());
        };
        "managed_symlink_removed"
    )]
    #[test_case(
        |target, db| {
            fs::write(target.join("file"), "data").unwrap();
            db.insert_or_update(&DbEntry {
                target_path: target.join("file"),
                deploy_type: DeployType::Copy,
                source_path: PathBuf::from("/src/file"),
                hash: Some(hash_file(&target.join("file")).unwrap()),
                symlink_target: None,
                mtime: None,
            }).unwrap();
        },
        false, false, false,
        |target, db| {
            assert!(!target.join("file").exists());
            assert!(db.get_entry(&target.join("file")).unwrap().is_none());
        };
        "managed_copy_removed"
    )]
    #[test_case(
        |target, db| {
            let source = target.parent().unwrap().join("source");
            fs::create_dir_all(&source).unwrap();
            unix_fs::symlink(Path::new("/a"), source.join("file")).unwrap();
            unix_fs::symlink(Path::new("/a"), target.join("link")).unwrap();
            db.insert_or_update(&DbEntry {
                target_path: target.join("link"),
                deploy_type: DeployType::Copy,
                source_path: source.join("file"),
                hash: None,
                symlink_target: Some(PathBuf::from("/a")),
                mtime: None,
            }).unwrap();
        },
        false, false, false,
        |target, db| {
            assert!(!target.join("link").is_symlink());
            assert!(db.get_entry(&target.join("link")).unwrap().is_none());
        };
        "managed_copy_symlink_source_removed"
    )]
    #[test_case(
        |target, db| {
            fs::write(target.join("file"), "modified").unwrap();
            db.insert_or_update(&DbEntry {
                target_path: target.join("file"),
                deploy_type: DeployType::Copy,
                source_path: PathBuf::from("/src/file"),
                hash: Some(999),
                symlink_target: None,
                mtime: None,
            }).unwrap();
        },
        false, false, false,
        |target, db| {
            assert!(target.join("file").exists());
            assert!(db.get_entry(&target.join("file")).unwrap().is_none());
        };
        "unmanaged_file_stays"
    )]
    #[test_case(
        |target, db| {
            db.insert_or_update(&DbEntry {
                target_path: target.join("ghost"),
                deploy_type: DeployType::Copy,
                source_path: PathBuf::from("/src/ghost"),
                hash: Some(0),
                symlink_target: None,
                mtime: None,
            }).unwrap();
        },
        false, false, false,
        |target, db| {
            assert!(!target.join("ghost").exists());
            assert!(db.get_entry(&target.join("ghost")).unwrap().is_none());
        };
        "missing_file_db_cleaned"
    )]
    #[test_case(
        |target, db| {
            fs::write(target.join("file"), "data").unwrap();
            db.insert_or_update(&DbEntry {
                target_path: target.join("file"),
                deploy_type: DeployType::Copy,
                source_path: PathBuf::from("/src/file"),
                hash: Some(hash_file(&target.join("file")).unwrap()),
                symlink_target: None,
                mtime: None,
            }).unwrap();
        },
        true, false, false,
        |target, db| {
            assert!(target.join("file").exists());
            assert!(db.get_entry(&target.join("file")).unwrap().is_some());
        };
        "portal_entry_skipped"
    )]
    #[test_case(
        |target, db| {
            fs::write(target.join("file"), "data").unwrap();
            db.insert_or_update(&DbEntry {
                target_path: target.join("file"),
                deploy_type: DeployType::Copy,
                source_path: PathBuf::from("/src/file"),
                hash: Some(hash_file(&target.join("file")).unwrap()),
                symlink_target: None,
                mtime: None,
            }).unwrap();
        },
        false, true, false,
        |target, db| {
            assert!(target.join("file").exists());
            assert!(db.get_entry(&target.join("file")).unwrap().is_some());
        };
        "dry_run_no_changes"
    )]
    #[test_case(
        |target, db| {
            fs::create_dir_all(target.join("a/b")).unwrap();
            fs::write(target.join("other"), "data").unwrap();
            fs::write(target.join("a/b/file"), "data").unwrap();
            db.insert_or_update(&DbEntry {
                target_path: target.join("a/b/file"),
                deploy_type: DeployType::Copy,
                source_path: PathBuf::from("/src/file"),
                hash: Some(hash_file(&target.join("a/b/file")).unwrap()),
                symlink_target: None,
                mtime: None,
            }).unwrap();
        },
        false, false, true,
        |target, db| {
            assert!(!target.join("a/b/file").exists());
            assert!(!target.join("a/b").exists());
            assert!(!target.join("a").exists());
            assert!(target.exists());
            assert!(db.get_entry(&target.join("a/b/file")).unwrap().is_none());
        };
        "prune_empty_dirs"
    )]
    #[test_case(
        |target, db| {
            fs::create_dir_all(target.join("a/b")).unwrap();
            fs::write(target.join("a/b/managed"), "data").unwrap();
            fs::write(target.join("a/other"), "keep").unwrap();
            db.insert_or_update(&DbEntry {
                target_path: target.join("a/b/managed"),
                deploy_type: DeployType::Copy,
                source_path: PathBuf::from("/src/file"),
                hash: Some(hash_file(&target.join("a/b/managed")).unwrap()),
                symlink_target: None,
                mtime: None,
            }).unwrap();
        },
        false, false, true,
        |target, db| {
            assert!(!target.join("a/b/managed").exists());
            assert!(!target.join("a/b").exists());
            assert!(target.join("a/other").exists());
            assert!(target.join("a").exists());
            assert!(db.get_entry(&target.join("a/b/managed")).unwrap().is_none());
        };
        "prune_stops_at_middle"
    )]
    #[test_case(
        |target, db| {
            fs::create_dir_all(target.join("a/b")).unwrap();
            fs::write(target.join("a/b/managed"), "data").unwrap();
            db.insert_or_update(&DbEntry {
                target_path: target.join("a/b/managed"),
                deploy_type: DeployType::Copy,
                source_path: PathBuf::from("/src/file"),
                hash: Some(hash_file(&target.join("a/b/managed")).unwrap()),
                symlink_target: None,
                mtime: None,
            }).unwrap();
        },
        false, true, true,
        |target, db| {
            assert!(target.join("a/b/managed").exists());
            assert!(target.join("a/b").exists());
            assert!(db.get_entry(&target.join("a/b/managed")).unwrap().is_some());
        };
        "dry_run_prune_no_changes"
    )]
    fn test_clean_up(
        setup: impl FnOnce(&Path, &Db),
        in_portal: bool,
        dry_run: bool,
        prune_empty_dirs: bool,
        assert: impl FnOnce(&Path, &Db),
    ) {
        let temp_dir = tempdir().unwrap();
        let target_dir = temp_dir.path().join("target");
        fs::create_dir_all(&target_dir).unwrap();
        let db = Db::init(&temp_dir.path().join("db")).unwrap();
        setup(&target_dir, &db);
        let portal_entries = if in_portal {
            let entries = db.get_all_entries().unwrap();
            Some(
                entries
                    .into_iter()
                    .map(|e| (e.target_path.clone(), PortalEntry::default()))
                    .collect(),
            )
        } else {
            None
        };
        let empty_portal = HashMap::new();
        let portal_ref = portal_entries.as_ref().unwrap_or(&empty_portal);
        clean_up(portal_ref, &db, dry_run, prune_empty_dirs, false, false).unwrap();
        assert(&target_dir, &db);
    }
}
