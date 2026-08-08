use std::fs::{self, File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StateError {
    #[error("state I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("state database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("state lock is already held")]
    LockContended,
    #[error("invalid state record: {0}")]
    InvalidRecord(String),
}

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

    fn parse(value: &str) -> Result<Self, StateError> {
        match value {
            "file" => Ok(Self::File),
            "symlink" => Ok(Self::Symlink),
            other => Err(StateError::InvalidRecord(other.to_owned())),
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

impl StateDatabase {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, StateError> {
        let root = root.as_ref();
        fs::create_dir_all(root)?;
        let path = root.join("state.sqlite");
        let connection = Connection::open(&path)?;
        connection.execute_batch(
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
        )?;
        Ok(Self { connection, path })
    }

    pub fn open_read_only(root: impl AsRef<Path>) -> Result<Self, StateError> {
        let root = root.as_ref();
        let path = root.join("state.sqlite");
        if !path.exists() {
            let connection = Connection::open_in_memory()?;
            connection.execute_batch(
                "CREATE TABLE managed_paths (
                    target_path TEXT PRIMARY KEY,
                    source_path TEXT NOT NULL,
                    kind TEXT NOT NULL CHECK (kind IN ('file', 'symlink')),
                    link_target TEXT,
                    content_hash TEXT,
                    CHECK ((kind = 'symlink' AND link_target IS NOT NULL AND content_hash IS NULL)
                        OR (kind = 'file' AND content_hash IS NOT NULL AND link_target IS NULL))
                )",
            )?;
            connection.execute_batch(
                "CREATE TABLE active_profiles (
                    name TEXT PRIMARY KEY,
                    activated_at INTEGER NOT NULL
                )",
            )?;
            return Ok(Self { connection, path });
        }
        let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        Ok(Self { connection, path })
    }

    pub fn managed_paths(&self) -> Result<Vec<StateRecord>, StateError> {
        let mut statement = self.connection.prepare(
            "SELECT target_path, source_path, kind, link_target, content_hash
             FROM managed_paths",
        )?;
        let rows = statement.query_map([], |row| {
            let kind = Kind::parse(&row.get::<_, String>(2)?)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            Ok(StateRecord {
                target_path: PathBuf::from(row.get::<_, String>(0)?),
                source_path: PathBuf::from(row.get::<_, String>(1)?),
                kind,
                link_target: row.get::<_, Option<String>>(3)?.map(PathBuf::from),
                content_hash: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StateError::from)
    }

    pub fn put(&self, record: &StateRecord) -> Result<(), StateError> {
        self.connection.execute(
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
        )?;
        Ok(())
    }

    pub fn contains(&self, target_path: &Path) -> Result<bool, StateError> {
        self.connection
            .query_row(
                "SELECT 1 FROM managed_paths WHERE target_path = ?1",
                [target_path.to_string_lossy()],
                |_| Ok(()),
            )
            .optional()
            .map(|value| value.is_some())
            .map_err(StateError::from)
    }
}

pub struct StateLock {
    file: File,
}

impl StateLock {
    pub fn acquire(root: impl AsRef<Path>) -> Result<Self, StateError> {
        let root = root.as_ref();
        fs::create_dir_all(root)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("state.lock"))?;
        // SAFETY: the file descriptor is open for the lifetime of this lock.
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result == -1 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EWOULDBLOCK) {
                return Err(StateError::LockContended);
            }
            return Err(std::io::Error::last_os_error().into());
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
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn database_persists_state_records_and_creates_schema() {
        let directory = tempdir().unwrap();
        let database = StateDatabase::open(directory.path()).unwrap();
        let record = StateRecord {
            target_path: PathBuf::from("/target"),
            source_path: PathBuf::from("/source"),
            kind: Kind::File,
            link_target: None,
            content_hash: Some("0123456789abcdef".into()),
        };
        database.put(&record).unwrap();

        let reopened = StateDatabase::open(directory.path()).unwrap();
        assert_eq!(reopened.managed_paths().unwrap(), vec![record]);
    }

    #[test]
    fn lock_fails_fast_when_another_lock_is_held() {
        let directory = tempdir().unwrap();
        let _lock = StateLock::acquire(directory.path()).unwrap();

        assert!(matches!(
            StateLock::acquire(directory.path()),
            Err(StateError::LockContended)
        ));
    }
}
