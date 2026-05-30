use std::{
    path::{Path, PathBuf},
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use miette::{Context, Result, miette};
use rusqlite::{Connection, OptionalExtension, params};

use crate::config::DeployType;

const TABLE_NAME: &str = "managed_files";
const PROFILES_TABLE: &str = "active_profiles";

pub struct Db {
    conn: Connection,
}

#[derive(Default, Debug, PartialEq)]
pub struct DbEntry {
    pub target_path: PathBuf,
    pub deploy_type: DeployType,
    pub source_path: PathBuf,
    pub hash: Option<u64>,
    pub symlink_target: Option<PathBuf>,
    pub mtime: Option<i64>,
}

pub struct ActiveProfile {
    pub name: String,
    pub activated_at: i64,
}

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<DbEntry> {
    let target_str: String = row.get(0)?;
    let deploy_str: String = row.get(1)?;
    let source_str: String = row.get(2)?;
    let hash_str: Option<String> = row.get(3)?;
    let symlink_target_str: Option<String> = row.get(4)?;

    let deploy_type = DeployType::from_str(&deploy_str).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            miette!("Invalid deploy type `{deploy_str}` for `{target_str}`").into(),
        )
    })?;

    let hash = hash_str.and_then(|s| u64::from_str_radix(&s, 16).ok());
    let mtime: Option<i64> = row.get(5)?;

    let symlink_target = symlink_target_str.map(PathBuf::from);

    Ok(DbEntry {
        target_path: PathBuf::from(target_str),
        deploy_type,
        source_path: PathBuf::from(source_str),
        hash,
        symlink_target,
        mtime,
    })
}

impl Db {
    pub fn init(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            crate::create_dir_err!(std::fs::create_dir_all(parent), parent)?;
        }

        let conn = Connection::open(path)
            .map_err(|e| miette!("{e}"))
            .wrap_err_with(|| format!("Failed to open connection at `{}`", path.display()))?;

        conn.execute(
            &format!(
                "CREATE TABLE IF NOT EXISTS {} (
                target_path TEXT PRIMARY KEY,
                deploy_type TEXT NOT NULL,
                source_path TEXT NOT NULL,
                hash TEXT,
                symlink_target TEXT,
                mtime INTEGER
            )",
                TABLE_NAME
            ),
            [],
        )
        .map_err(|e| miette!("{e}"))
        .wrap_err("Failed to initialize database")?;

        conn.execute(
            &format!(
                "CREATE TABLE IF NOT EXISTS {} (
                activated_at INTEGER NOT NULL,
                name TEXT NOT NULL UNIQUE
            )",
                PROFILES_TABLE
            ),
            [],
        )
        .map_err(|e| miette!("{e}"))
        .wrap_err("Failed to initialize profile table")?;

        Ok(Self { conn })
    }

    pub fn insert_or_update(&self, entry: &DbEntry) -> Result<()> {
        let hash_str = entry.hash.map(|h| format!("{:x}", h));
        let symlink_target_str = entry
            .symlink_target
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());

        self.conn
            .execute(
                &format!("INSERT OR REPLACE INTO {} (target_path, deploy_type, source_path, hash, symlink_target, mtime) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", TABLE_NAME),
                params![
                    entry.target_path.to_string_lossy(),
                    entry.deploy_type.to_string(),
                    entry.source_path.to_string_lossy(),
                    hash_str,
                    symlink_target_str,
                    entry.mtime,
                ],
            )
            .map_err(|e| miette!("{e}"))
            .wrap_err_with(|| format!("Failed to insert/update entry for `{}`", entry.target_path.display()))?;

        Ok(())
    }

    pub fn delete_entry(&self, target: &Path) -> Result<()> {
        self.conn
            .execute(
                &format!("DELETE FROM {} WHERE target_path = ?1", TABLE_NAME),
                params![target.to_string_lossy()],
            )
            .map_err(|e| miette!("{e}"))
            .wrap_err_with(|| format!("Failed to delete entry for `{}`", target.display()))?;
        Ok(())
    }

    pub fn delete_entry_with_prefix(&self, target: &Path) -> Result<()> {
        let prefix = target.to_string_lossy();
        let upper = format!("{}\u{10FFFF}", prefix);
        self.conn
            .execute(
                &format!(
                    "DELETE FROM {} WHERE target_path >= ?1 AND target_path < ?2",
                    TABLE_NAME
                ),
                params![prefix.as_ref(), upper],
            )
            .map_err(|e| miette!("{e}"))
            .wrap_err_with(|| {
                format!(
                    "Failed to delete entries with prefix `{}`",
                    target.display()
                )
            })?;
        Ok(())
    }

    pub fn delete_table(&self) -> Result<()> {
        self.conn
            .execute(&format!("DROP TABLE IF EXISTS {}", TABLE_NAME), [])
            .map_err(|e| miette!("{e}"))
            .wrap_err("Failed to clear database")?;
        Ok(())
    }

    pub fn get_entry(&self, target: &Path) -> Result<Option<DbEntry>> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT target_path, deploy_type, source_path, hash, symlink_target, mtime FROM {} WHERE target_path = ?1",
                TABLE_NAME
            ))
            .map_err(|e| miette!("{e}"))
            .wrap_err_with(|| format!("Failed to look up `{}`", target.display()))?;

        stmt.query_row(params![target.to_string_lossy()], row_to_entry)
            .optional()
            .map_err(|e| miette!("{e}"))
            .wrap_err_with(|| format!("Failed to query entry for `{}`", target.display()))
    }

    pub fn get_all_entries(&self) -> Result<Vec<DbEntry>> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT target_path, deploy_type, source_path, hash, symlink_target, mtime FROM {}",
                TABLE_NAME
            ))
            .map_err(|e| miette!("{e}"))
            .wrap_err("Failed to list database entries")?;

        let rows = stmt
            .query_map([], row_to_entry)
            .map_err(|e| miette!("{e}"))
            .wrap_err("Failed to query entries from database")?;
        let mut result = Vec::new();
        for entry in rows {
            result.push(
                entry
                    .map_err(|e| miette!("{e}"))
                    .wrap_err("Failed to read database entry")?,
            );
        }
        Ok(result)
    }

    pub fn activate_profile(&self, name: &str) -> Result<()> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| miette!("{e}"))
            .wrap_err("System clock is before epoch")?
            .as_millis() as i64;

        self.conn
            .execute(
                &format!(
                    "INSERT OR REPLACE INTO {} (name, activated_at) VALUES (?1, ?2)",
                    PROFILES_TABLE
                ),
                params![name, now_ms],
            )
            .map_err(|e| miette!("{e}"))
            .wrap_err_with(|| format!("Failed to activate profile `{name}`"))?;
        Ok(())
    }

    pub fn deactivate_profile(&self, name: &str) -> Result<()> {
        self.conn
            .execute(
                &format!("DELETE FROM {} WHERE name = ?1", PROFILES_TABLE),
                params![name],
            )
            .map_err(|e| miette!("{e}"))
            .wrap_err_with(|| format!("Failed to deactivate profile `{name}`"))?;
        Ok(())
    }

    pub fn get_active_profiles(&self) -> Result<Vec<ActiveProfile>> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT name, activated_at FROM {} ORDER BY activated_at ASC",
                PROFILES_TABLE
            ))
            .map_err(|e| miette!("{e}"))
            .wrap_err("Failed to query active profiles")?;

        let rows = stmt
            .query_map([], |row| {
                Ok(ActiveProfile {
                    name: row.get(0)?,
                    activated_at: row.get(1)?,
                })
            })
            .map_err(|e| miette!("{e}"))
            .wrap_err("Failed to query active profiles")?;

        let mut result = Vec::new();
        for profile in rows {
            result.push(
                profile
                    .map_err(|e| miette!("{e}"))
                    .wrap_err("Failed to read active profile")?,
            );
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use test_case::test_case;

    #[test]
    fn test_db_init() {
        let temp_dir = tempdir().unwrap();
        let path = &temp_dir.path().join("db");
        let _db1 = Db::init(path).unwrap();
        let _db2 = Db::init(path).unwrap();
        let _db3 = Db::init(path).unwrap();
    }

    #[test_case(
        |db: &Db| {
            db.insert_or_update(&DbEntry::default()).unwrap();
        },
        |db: &Db| {
            assert_eq!(db.get_entry(&PathBuf::new()).unwrap(), Some(DbEntry::default()));
        };
        "insert"
    )]
    #[test_case(
        |db: &Db| {
            for p in 'a'..='z' {
                db.insert_or_update(&DbEntry {
                    target_path: PathBuf::from(p.to_string()),
                    ..Default::default()
                })
                .unwrap();
            }
        },
        |db: &Db| {
            assert_eq!(db.get_all_entries().unwrap().len(), 26);
        };
        "multiple_insert"
    )]
    #[test_case(
        |db: &Db| {
            let entry = DbEntry::default();
            db.insert_or_update(&entry).unwrap();
            db.insert_or_update(&DbEntry { hash: Some(1), ..entry }).unwrap();
        },
        |db: &Db| {
            assert_eq!(
                db.get_entry(&PathBuf::new()).unwrap(),
                Some(DbEntry { hash: Some(1), ..Default::default() })
            );
            assert_eq!(db.get_all_entries().unwrap().len(), 1);
        };
        "update"
    )]
    #[test_case(
        |db: &Db| {
            db.insert_or_update(&DbEntry::default()).unwrap();
        },
        |db: &Db| {
            assert!(db.get_entry(&PathBuf::new()).unwrap().is_some());
            db.delete_entry(Path::new("")).unwrap();
            assert!(db.get_entry(&PathBuf::new()).unwrap().is_none());
        };
        "delete"
    )]
    #[test_case(
        |db: &Db| {
            db.insert_or_update(&DbEntry { target_path: PathBuf::from("/a/b"), ..Default::default() }).unwrap();
            db.insert_or_update(&DbEntry { target_path: PathBuf::from("/a/c"), ..Default::default() }).unwrap();
            db.insert_or_update(&DbEntry { target_path: PathBuf::from("/b/a"), ..Default::default() }).unwrap();
        },
        |db: &Db| {
            db.delete_entry_with_prefix(Path::new("/ab")).unwrap();
            assert_eq!(db.get_all_entries().unwrap().len(), 3);
            db.delete_entry_with_prefix(Path::new("/a")).unwrap();
            assert_eq!(db.get_all_entries().unwrap().len(), 1);
        };
        "delete_with_prefix"
    )]
    #[test_case(
        |_: &Db| {},
        |db: &Db| {
            db.delete_table().unwrap();
        };
        "delete_table"
    )]
    #[test_case(
        |db: &Db| {
            db.activate_profile("foo").unwrap();
        },
        |db: &Db| {
            let profiles = db.get_active_profiles().unwrap();
            assert_eq!(profiles.len(), 1);
            assert_eq!(profiles[0].name, "foo");
            assert!(profiles[0].activated_at > 0);
        };
        "activate_profile"
    )]
    #[test_case(
        |db: &Db| {
            db.activate_profile("a").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10));
            db.activate_profile("b").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10));
            db.activate_profile("c").unwrap();
        },
        |db: &Db| {
            let profiles = db.get_active_profiles().unwrap();
            assert_eq!(profiles.len(), 3);
            let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
            assert_eq!(names, vec!["a", "b", "c"]);
        };
        "activate_multiple"
    )]
    #[test_case(
        |db: &Db| {
            db.activate_profile("foo").unwrap();
        },
        |db: &Db| {
            let first = db.get_active_profiles().unwrap()[0].activated_at;
            std::thread::sleep(std::time::Duration::from_millis(10));
            db.activate_profile("foo").unwrap();
            let profiles = db.get_active_profiles().unwrap();
            assert_eq!(profiles.len(), 1);
            assert_eq!(profiles[0].name, "foo");
            assert!(profiles[0].activated_at > first);
        };
        "reactivate_profile"
    )]
    #[test_case(
        |db: &Db| {
            db.activate_profile("foo").unwrap();
        },
        |db: &Db| {
            assert_eq!(db.get_active_profiles().unwrap().len(), 1);
            db.deactivate_profile("foo").unwrap();
            assert!(db.get_active_profiles().unwrap().is_empty());
        };
        "deactivate_profile"
    )]
    #[test_case(
        |_: &Db| {},
        |db: &Db| {
            db.deactivate_profile("nope").unwrap();
            assert!(db.get_active_profiles().unwrap().is_empty());
        };
        "deactivate_nonexistent"
    )]
    #[test_case(
        |_: &Db| {},
        |db: &Db| {
            assert!(db.get_active_profiles().unwrap().is_empty());
        };
        "get_active_profiles_empty"
    )]
    fn test_db(setup: impl FnOnce(&Db), assert: impl FnOnce(&Db)) {
        let temp_dir = tempdir().unwrap();
        let db = Db::init(&temp_dir.path().join("db")).unwrap();
        setup(&db);
        assert(&db);
    }
}
