use std::{
    fs::{self, File, OpenOptions},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use miette::{Result, WrapErr, miette};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Symlink,
}

impl Kind {
    pub(crate) fn as_str(self) -> &'static str {
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
    pub content_hash: Option<String>,
}

#[cfg(any(test, feature = "testing"))]
pub mod test_hooks {
    use std::{cell::RefCell, path::PathBuf};

    #[macro_export]
    macro_rules! record {
        (f, $target:expr, $hash:expr) => {
            $crate::state::StateRecord {
                target_path: std::path::PathBuf::from($target),
                source_path: std::path::PathBuf::new(),
                kind: $crate::state::Kind::File,
                content_hash: Some($hash.into()),
            }
        };
        (s, $target:expr, $source:expr) => {
            $crate::state::StateRecord {
                target_path: std::path::PathBuf::from($target),
                source_path: std::path::PathBuf::from($source),
                kind: $crate::state::Kind::Symlink,
                content_hash: None,
            }
        };
    }

    thread_local! {
        pub static TEST_STATE_ROOT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }
}

fn state_root() -> Result<PathBuf> {
    #[cfg(any(test, feature = "testing"))]
    {
        Ok(test_hooks::TEST_STATE_ROOT
            .with(|root| root.borrow().clone())
            .unwrap())
    }

    #[cfg(not(any(test, feature = "testing")))]
    {
        let state_home = dirs::state_dir()
            .or_else(dirs::data_dir)
            .map(|state_home| state_home.join("dotrift"))
            .ok_or_else(|| miette!("XDG_STATE_HOME and XDG_DATA_HOME are unset"))
            .wrap_err("cannot resolve state location")?;
        Ok(state_home)
    }
}

pub struct StateDatabase {
    connection: Connection,
    pub path: PathBuf,
}

impl StateDatabase {
    pub fn open() -> Result<Self> {
        Self::open_at(&state_root()?)
    }

    pub(crate) fn open_at(root: &Path) -> Result<Self> {
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
                content_hash TEXT,
                CHECK ((kind = 'symlink' AND content_hash IS NULL)
                    OR (kind = 'file' AND content_hash IS NOT NULL))
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
                "SELECT target_path, source_path, kind, content_hash
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
                    row.get(3)?,
                ))
            })
            .map_err(|error| miette!(error))
            .wrap_err("cannot read managed paths")?;
        let tuples = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| miette!(error))
            .wrap_err("cannot read managed paths")?;
        let mut records = Vec::with_capacity(tuples.len());
        for (target_path, source_path, kind, content_hash) in tuples {
            records.push(StateRecord {
                target_path,
                source_path,
                kind: Kind::parse(&kind)?,
                content_hash,
            });
        }
        Ok(records)
    }

    pub fn put(&self, record: &StateRecord) -> Result<()> {
        self.connection
            .execute(
                "INSERT OR REPLACE INTO managed_paths
                 (target_path, source_path, kind, content_hash)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    record.target_path.to_string_lossy(),
                    record.source_path.to_string_lossy(),
                    record.kind.as_str(),
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

    /// Returns the state record for an absolute target path, if one exists.
    pub fn record(&self, target_path: &Path) -> Result<Option<StateRecord>> {
        let record = self
            .connection
            .query_row(
                "SELECT target_path, source_path, kind, content_hash
                 FROM managed_paths WHERE target_path = ?1",
                [target_path.to_string_lossy()],
                |row| {
                    Ok((
                        PathBuf::from(row.get::<_, String>(0)?),
                        PathBuf::from(row.get::<_, String>(1)?),
                        row.get::<_, String>(2)?,
                        row.get(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| miette!(error))
            .wrap_err_with(|| {
                format!("cannot read state record for `{}`", target_path.display())
            })?;
        record
            .map(|(target_path, source_path, kind, content_hash)| {
                Ok(StateRecord {
                    target_path,
                    source_path,
                    kind: Kind::parse(&kind)?,
                    content_hash,
                })
            })
            .transpose()
    }

    /// Removes the state record for a target path, if present.
    pub fn remove(&self, target_path: &Path) -> Result<()> {
        self.connection
            .execute(
                "DELETE FROM managed_paths WHERE target_path = ?1",
                [target_path.to_string_lossy()],
            )
            .map_err(|error| miette!(error))
            .wrap_err_with(|| {
                format!("cannot remove state record for `{}`", target_path.display())
            })?;
        Ok(())
    }

    pub fn active_profiles(&self) -> Result<Vec<(String, i64)>> {
        let mut statement = self
            .connection
            .prepare("SELECT name, activated_at FROM active_profiles ORDER BY activated_at, name")
            .map_err(|error| miette!(error))
            .wrap_err("cannot read active profiles")?;
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|error| miette!(error))
            .wrap_err("cannot read active profiles")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| miette!(error))
            .wrap_err("cannot read active profiles")
    }

    pub fn activate_profile(&self, name: &str) -> Result<()> {
        let current: Option<i64> = self
            .connection
            .query_row("SELECT MAX(activated_at) FROM active_profiles", [], |row| {
                row.get(0)
            })
            .map_err(|error| miette!(error))
            .wrap_err("cannot read profile activation timestamps")?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| miette!(error))
            .wrap_err("system clock is before the Unix epoch")?
            .as_millis() as i64;
        let activated_at = now.max(current.map_or(0, |value| value.saturating_add(1)));
        self.connection
            .execute(
                "INSERT OR REPLACE INTO active_profiles (name, activated_at) VALUES (?1, ?2)",
                params![name, activated_at],
            )
            .map_err(|error| miette!(error))
            .wrap_err("cannot activate profile")?;
        Ok(())
    }

    pub fn deactivate_profile(&self, name: &str) -> Result<bool> {
        let count = self
            .connection
            .execute("DELETE FROM active_profiles WHERE name = ?1", [name])
            .map_err(|error| miette!(error))
            .wrap_err("cannot deactivate profile")?;
        Ok(count == 1)
    }
}

pub(crate) struct StateLock {
    file: File,
}

impl StateLock {
    pub(crate) fn acquire() -> Result<Self> {
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

    #[test_case(Kind::File, "file"; "file_kind_renders_as_file")]
    #[test_case(Kind::Symlink, "symlink"; "symlink_kind_renders_as_symlink")]
    fn kind_renders_to_string(kind: Kind, expected: &str) {
        assert_eq!(kind.as_str(), expected);
        assert_eq!(kind.to_string(), expected);
    }

    #[test_case("file" => matches Ok(Kind::File); "file_string_parses_to_file_kind")]
    #[test_case("symlink" => matches Ok(Kind::Symlink); "symlink_string_parses_to_symlink_kind")]
    #[test_case("link" => matches Err(_); "unknown_string_is_rejected")]
    fn kind_parses_from_string(value: &str) -> miette::Result<Kind> {
        Kind::parse(value)
    }

    #[test_case(crate::record!(f, "/home/user/.gitconfig", "abc123"); "file_record_round_trips")]
    #[test_case(crate::record!(s, "/home/user/.bashrc", "dotfiles/bash/bashrc"); "symlink_record_round_trips")]
    fn put_round_trips_record(record: StateRecord) {
        let (_dir, database) = database();
        database.put(&record).expect("cannot store record");
        assert_eq!(
            database.managed_paths().expect("cannot read records"),
            vec![record]
        );
    }

    #[test_case(
        crate::record!(f, "/home/user/.gitconfig", "abc123"),
        crate::record!(s, "/home/user/.gitconfig", "dotfiles/git/global");
        "file_record_overwritten_by_symlink"
    )]
    #[test_case(
        crate::record!(s, "/home/user/.bashrc", "dotfiles/bash/bashrc"),
        crate::record!(f, "/home/user/.bashrc", "deadbeef");
        "symlink_record_overwritten_by_file"
    )]
    fn put_overwrites_record_at_same_target_path(original: StateRecord, replacement: StateRecord) {
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

    #[test_case(
        StateRecord {
            target_path: PathBuf::from("/home/user/x"),
            source_path: PathBuf::from("dotfiles/x"),
            kind: Kind::File,
            content_hash: None,
        };
        "file_record_without_content_hash_rejected"
    )]
    #[test_case(
        StateRecord {
            target_path: PathBuf::from("/home/user/y"),
            source_path: PathBuf::from("dotfiles/y"),
            kind: Kind::Symlink,
            content_hash: Some("abc".into()),
        };
        "symlink_record_with_content_hash_rejected"
    )]
    fn put_rejects_schema_violating_record(record: StateRecord) {
        let (_dir, database) = database();
        assert!(database.put(&record).is_err());
    }

    #[test]
    fn managed_paths_returns_every_stored_record() {
        let (_dir, database) = database();
        let first = crate::record!(f, "/home/user/.gitconfig", "abc123");
        let second = crate::record!(s, "/home/user/.bashrc", "dotfiles/bash/bashrc");
        database.put(&first).expect("cannot store first record");
        database.put(&second).expect("cannot store second record");
        let mut records = database.managed_paths().expect("cannot read records");
        let mut expected = vec![first, second];
        records.sort_by(|left, right| left.target_path.cmp(&right.target_path));
        expected.sort_by(|left, right| left.target_path.cmp(&right.target_path));
        assert_eq!(records, expected);
    }

    #[test_case(crate::record!(f, "/home/user/.gitconfig", "abc123"); "file_record_found_by_target_path")]
    #[test_case(crate::record!(s, "/home/user/.bashrc", "dotfiles/bash/bashrc"); "symlink_record_found_by_target_path")]
    fn record_lookup_returns_stored_record(record: StateRecord) {
        let (_dir, database) = database();
        database.put(&record).expect("cannot store record");
        assert_eq!(
            database
                .record(&record.target_path)
                .expect("cannot read record"),
            Some(record)
        );
    }

    #[test]
    fn record_lookup_returns_none_for_unknown_target_path() {
        let (_dir, database) = database();
        assert_eq!(
            database
                .record(Path::new("/home/user/.nonexistent"))
                .expect("cannot read record"),
            None
        );
    }

    #[test]
    fn record_lookup_errors_on_unknown_kind_in_database() {
        let (_dir, database) = database();
        database
            .connection
            .execute_batch(
                "DROP TABLE managed_paths;
                 CREATE TABLE managed_paths (
                     target_path TEXT PRIMARY KEY,
                     source_path TEXT NOT NULL,
                     kind TEXT NOT NULL,
                     content_hash TEXT
                 );
                 INSERT INTO managed_paths VALUES
                     ('/home/user/.gitconfig', 'dotfiles/git/config', 'link', 'abc123');",
            )
            .expect("cannot seed corrupt record");
        assert!(database.record(Path::new("/home/user/.gitconfig")).is_err());
    }

    #[test_case(crate::record!(f, "/home/user/.gitconfig", "abc123"); "file_record_deleted")]
    #[test_case(crate::record!(s, "/home/user/.bashrc", "dotfiles/bash/bashrc"); "symlink_record_deleted")]
    fn remove_deletes_stored_record(record: StateRecord) {
        let (_dir, database) = database();
        database.put(&record).expect("cannot store record");
        database
            .remove(&record.target_path)
            .expect("cannot remove record");
        assert_eq!(
            database.managed_paths().expect("cannot read records"),
            vec![]
        );
    }

    #[test]
    fn remove_succeeds_for_unknown_target_path() {
        let (_dir, database) = database();
        database
            .remove(Path::new("/home/user/.nonexistent"))
            .expect("cannot remove absent record");
    }

    #[test]
    fn active_profiles_returns_empty_before_any_activation() {
        let (_dir, database) = database();
        assert_eq!(
            database.active_profiles().expect("cannot read profiles"),
            vec![]
        );
    }

    #[test]
    fn active_profiles_orders_by_activation_then_name() {
        let (_dir, database) = database();
        database
            .activate_profile("editor")
            .expect("cannot activate profile");
        database
            .activate_profile("shell")
            .expect("cannot activate profile");
        database
            .activate_profile("editor")
            .expect("cannot reactivate profile");
        let profiles = database.active_profiles().expect("cannot read profiles");
        assert_eq!(
            profiles
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            vec!["shell", "editor"]
        );
    }

    #[test]
    fn activate_profile_updates_timestamp_without_duplicate_row() {
        let (_dir, database) = database();
        database
            .activate_profile("editor")
            .expect("cannot activate profile");
        database
            .activate_profile("editor")
            .expect("cannot reactivate profile");
        let profiles = database.active_profiles().expect("cannot read profiles");
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].0, "editor");
    }

    #[test]
    fn deactivate_profile_returns_true_for_active_profile() {
        let (_dir, database) = database();
        database
            .activate_profile("editor")
            .expect("cannot activate profile");
        assert!(
            database
                .deactivate_profile("editor")
                .expect("cannot deactivate profile")
        );
    }

    #[test]
    fn deactivate_profile_returns_false_for_absent_profile() {
        let (_dir, database) = database();
        assert!(
            !database
                .deactivate_profile("editor")
                .expect("cannot deactivate profile")
        );
    }

    #[test]
    fn state_lock_blocks_second_acquire_until_dropped() {
        let dir = tempfile::tempdir().expect("cannot create temp dir");
        let first = StateLock::acquire_at(dir.path()).expect("cannot acquire first lock");
        assert!(StateLock::acquire_at(dir.path()).is_err());
        drop(first);
        assert!(StateLock::acquire_at(dir.path()).is_ok());
    }
}
