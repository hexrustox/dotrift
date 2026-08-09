use std::fs::{self, File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use miette::{Result, miette};
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
            other => Err(miette!("invalid state record: {other}")),
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

pub struct StateDatabase {
    connection: Connection,
    pub path: PathBuf,
}

pub(crate) fn state_root() -> Result<PathBuf> {
    dirs::state_dir()
        .or_else(dirs::data_dir)
        .map(|state_home| state_home.join("dotrift"))
        .ok_or_else(|| {
            miette!("cannot resolve state location: XDG_STATE_HOME and XDG_DATA_HOME are unset")
        })
}

impl StateDatabase {
    pub fn open() -> Result<Self> {
        Self::open_at(&state_root()?)
    }

    fn open_at(root: &Path) -> Result<Self> {
        fs::create_dir_all(root).map_err(|error| miette!(error))?;
        let path = root.join("state.sqlite");
        let connection = Connection::open(&path).map_err(|error| miette!(error))?;
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
            .map_err(|error| miette!(error))?;
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
            .map_err(|error| miette!(error))?;
        Ok(Some(Self { connection, path }))
    }

    pub fn managed_paths(&self) -> Result<Vec<StateRecord>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT target_path, source_path, kind, link_target, content_hash
                 FROM managed_paths",
            )
            .map_err(|error| miette!(error))?;
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
            .map_err(|error| miette!(error))?;
        let tuples = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| miette!(error))?;
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
            .map_err(|error| miette!(error))?;
        Ok(())
    }

    pub fn contains(&self, target_path: &Path) -> Result<bool> {
        self.connection
            .query_row(
                "SELECT 1 FROM managed_paths WHERE target_path = ?1",
                [target_path.to_string_lossy()],
                |_| Ok(()),
            )
            .optional()
            .map(|value| value.is_some())
            .map_err(|error| miette!(error))
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
        fs::create_dir_all(root).map_err(|error| miette!(error))?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("state.lock"))
            .map_err(|error| miette!(error))?;
        // SAFETY: the file descriptor is open for the lifetime of this lock.
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result == -1 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
                return Err(miette!("state lock is already held"));
            }
            return Err(miette!(error));
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
