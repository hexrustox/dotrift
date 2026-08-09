use std::fs::{self, File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use miette::{Result, WrapErr, miette};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Symlink,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Symlink => "symlink",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "file" => Ok(Self::File),
            "symlink" => Ok(Self::Symlink),
            other => Err(miette::MietteDiagnostic::new(format!(
                "unknown state record kind `{other}`"
            ))
            .with_help("this is likely an internal error")
            .into()),
        }
    }
}

impl std::fmt::Display for Kind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str((*self).as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateRecord {
    pub target_path: PathBuf,
    pub source_path: PathBuf,
    pub kind: Kind,
    pub link_target: Option<PathBuf>,
    pub content_hash: Option<String>,
}

#[cfg(test)]
#[macro_export]
macro_rules! record {
    (f, $target:expr, $source:expr, $hash:expr) => {
        StateRecord {
            target_path: PathBuf::from($target),
            source_path: PathBuf::from($source),
            kind: Kind::File,
            link_target: None,
            content_hash: Some($hash.into()),
        }
    };
    (s, $target:expr, $source:expr, $link_target:expr) => {
        StateRecord {
            target_path: PathBuf::from($target),
            source_path: PathBuf::from($source),
            kind: Kind::Symlink,
            link_target: Some(PathBuf::from($link_target)),
            content_hash: None,
        }
    };
}

pub struct StateDatabase {
    connection: Connection,
    pub path: PathBuf,
}

pub(crate) fn state_root() -> Result<PathBuf> {
    let state_home = dirs::state_dir()
        .or_else(dirs::data_dir)
        .map(|state_home| state_home.join("dotrift"))
        .ok_or_else(|| miette!("XDG_STATE_HOME and XDG_DATA_HOME are unset"))
        .wrap_err("cannot resolve state location")?;
    Ok(state_home)
}

impl StateDatabase {
    pub fn open() -> Result<Self> {
        Self::open_at(&state_root()?)
    }

    fn open_at(root: &Path) -> Result<Self> {
        fs::create_dir_all(root)
            .map_err(|error| miette!(error))
            .wrap_err_with(|| format!("cannot create state directory `{}`", root.display()))?;
        let path = root.join("state.sqlite");
        let connection = Connection::open(&path)
            .map_err(|error| miette!(error))
            .wrap_err_with(|| format!("cannot open state database `{}`", path.display()))?;
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS managed_paths (
                target_path TEXT PRIMARY KEY,
                source_path TEXT NOT NULL,
                kind TEXT NOT NULL CHECK (kind IN ('file', 'symlink')),
                link_target TEXT,
                content_hash TEXT,
                CHECK ((kind = 'symlink' AND link_target IS NOT NULL AND content_hash IS NULL)
                    OR (kind = 'file' AND content_hash IS NOT NULL AND link_target IS NULL))
            );
            CREATE TABLE IF NOT EXISTS active_profiles (
                name TEXT PRIMARY KEY,
                activated_at INTEGER NOT NULL
            );",
            )
            .map_err(|error| miette!(error))
            .wrap_err("cannot initialize state schema")?;
        Ok(Self { connection, path })
    }

    pub fn open_read_only() -> Result<Option<Self>> {
        Self::open_read_only_at(&state_root()?)
    }

    fn open_read_only_at(root: &Path) -> Result<Option<Self>> {
        let path = root.join("state.sqlite");
        if !path.exists() {
            return Ok(None);
        }
        let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| miette!(error))
            .wrap_err_with(|| format!("cannot open state database `{}`", path.display()))?;
        Ok(Some(Self { connection, path }))
    }

    pub fn managed_paths(&self) -> Result<Vec<StateRecord>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT target_path, source_path, kind, link_target, content_hash
                 FROM managed_paths",
            )
            .map_err(|error| miette!(error))
            .wrap_err("cannot read managed paths")?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    PathBuf::from(row.get::<_, String>(0)?),
                    PathBuf::from(row.get::<_, String>(1)?),
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?.map(PathBuf::from),
                    row.get(4)?,
                ))
            })
            .map_err(|error| miette!(error))
            .wrap_err("cannot read managed paths")?;
        let tuples = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| miette!(error))
            .wrap_err("cannot read managed paths")?;
        let mut records = Vec::with_capacity(tuples.len());
        for (target_path, source_path, kind, link_target, content_hash) in tuples {
            records.push(StateRecord {
                target_path,
                source_path,
                kind: Kind::parse(&kind)?,
                link_target,
                content_hash,
            });
        }
        Ok(records)
    }

    pub fn put(&self, record: &StateRecord) -> Result<()> {
        self.connection
            .execute(
                "INSERT OR REPLACE INTO managed_paths
                 (target_path, source_path, kind, link_target, content_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    record.target_path.to_string_lossy(),
                    record.source_path.to_string_lossy(),
                    record.kind.as_str(),
                    record
                        .link_target
                        .as_ref()
                        .map(|path| path.to_string_lossy()),
                    record.content_hash,
                ],
            )
            .map_err(|error| miette!(error))
            .wrap_err_with(|| {
                format!(
                    "cannot store state record for `{}`",
                    record.target_path.display()
                )
            })?;
        Ok(())
    }

    pub fn contains(&self, target_path: &Path) -> Result<bool> {
        let found = self
            .connection
            .query_row(
                "SELECT 1 FROM managed_paths WHERE target_path = ?1",
                [target_path.to_string_lossy()],
                |_| Ok(()),
            )
            .optional()
            .map(|value| value.is_some())
            .map_err(|error| miette!(error))
            .wrap_err_with(|| {
                format!("cannot check state record for `{}`", target_path.display())
            })?;
        Ok(found)
    }
}

pub struct StateLock {
    file: File,
}

impl StateLock {
    pub fn acquire() -> Result<Self> {
        Self::acquire_at(&state_root()?)
    }

    fn acquire_at(root: &Path) -> Result<Self> {
        fs::create_dir_all(root)
            .map_err(|error| miette!(error))
            .wrap_err_with(|| format!("cannot create state directory `{}`", root.display()))?;
        let lock_path = root.join("state.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|error| miette!(error))
            .wrap_err_with(|| format!("cannot open state lock `{}`", lock_path.display()))?;
        // SAFETY: the file descriptor is open for the lifetime of this lock.
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result == -1 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
                return Err(miette!("state lock is already held"));
            }
            Err(miette!(error)).wrap_err("cannot acquire state lock")?;
        }
        Ok(Self { file })
    }
}

impl Drop for StateLock {
    fn drop(&mut self) {
        // SAFETY: the descriptor came from the lock's still-open file.
        unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;

    fn database() -> (tempfile::TempDir, StateDatabase) {
        let dir = tempfile::tempdir().expect("cannot create temp dir");
        let database = StateDatabase::open_at(dir.path()).expect("cannot open test database");
        (dir, database)
    }

    #[test_case(Kind::File, "file"; "file")]
    #[test_case(Kind::Symlink, "symlink"; "symlink")]
    fn kind_as_str_and_display(kind: Kind, expected: &str) {
        assert_eq!(kind.as_str(), expected);
        assert_eq!(kind.to_string(), expected);
    }

    #[test_case("file" => matches Ok(Kind::File); "parses_file")]
    #[test_case("symlink" => matches Ok(Kind::Symlink); "parses_symlink")]
    #[test_case("link" => matches Err(_); "rejects_unknown_kind")]
    fn kind_parse(value: &str) -> miette::Result<Kind> {
        Kind::parse(value)
    }

    #[test_case(crate::record!(f, "/home/user/.gitconfig", "dotfiles/git/config", "abc123"); "file")]
    #[test_case(crate::record!(s, "/home/user/.bashrc", "dotfiles/bash/bashrc", "/home/user/.config/bashrc"); "symlink")]
    fn put_round_trips(record: StateRecord) {
        let (_dir, database) = database();
        database.put(&record).expect("cannot store record");
        assert_eq!(
            database.managed_paths().expect("cannot read records"),
            vec![record]
        );
    }

    #[test_case(
        crate::record!(f, "/home/user/.gitconfig", "dotfiles/git/config", "abc123"),
        crate::record!(s, "/home/user/.gitconfig", "dotfiles/git/global", "/home/user/.config/git/config");
        "file replaced by symlink"
    )]
    #[test_case(
        crate::record!(s, "/home/user/.bashrc", "dotfiles/bash/bashrc", "/home/user/.config/bashrc"),
        crate::record!(f, "/home/user/.bashrc", "dotfiles/bash/rc", "deadbeef");
        "symlink replaced by file"
    )]
    fn put_replaces_existing_record(original: StateRecord, replacement: StateRecord) {
        let (_dir, database) = database();
        database
            .put(&original)
            .expect("cannot store original record");
        database
            .put(&replacement)
            .expect("cannot store replacement record");
        assert_eq!(
            database.managed_paths().expect("cannot read records"),
            vec![replacement]
        );
    }

    #[test]
    fn contains_reports_stored_and_unknown() {
        let (_dir, database) = database();
        let record = crate::record!(f, "/home/user/.gitconfig", "dotfiles/git/config", "abc123");
        database.put(&record).expect("cannot store record");
        assert!(
            database
                .contains(&record.target_path)
                .expect("cannot check stored path")
        );
        assert!(
            !database
                .contains(Path::new("/home/user/.bashrc"))
                .expect("cannot check unknown path")
        );
    }

    #[test]
    fn state_lock_acquire_is_exclusive() {
        let dir = tempfile::tempdir().expect("cannot create temp dir");
        let first = StateLock::acquire_at(dir.path()).expect("cannot acquire first lock");
        assert!(StateLock::acquire_at(dir.path()).is_err());
        drop(first);
        assert!(StateLock::acquire_at(dir.path()).is_ok());
    }
}
