use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, eyre};
use rusqlite::{Connection, OptionalExtension, params};

use crate::config::DeployType;

const TABLE_NAME: &str = "managed_files";

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
}

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<DbEntry> {
    let target_str: String = row.get(0)?;
    let deploy_str: String = row.get(1)?;
    let source_str: String = row.get(2)?;
    let hash_str: Option<String> = row.get(3)?;
    let symlink_target_str: Option<String> = row.get(4)?;

    let deploy_type = match deploy_str.as_str() {
        "symlink" => DeployType::Symlink,
        "copy" => DeployType::Copy,
        other => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                eyre!("Invalid deploy type '{other}' for '{target_str}'").into(),
            ));
        }
    };

    let hash = hash_str.and_then(|s| u64::from_str_radix(&s, 16).ok());
    let symlink_target = symlink_target_str.map(PathBuf::from);

    Ok(DbEntry {
        target_path: PathBuf::from(target_str),
        deploy_type,
        source_path: PathBuf::from(source_str),
        hash,
        symlink_target,
    })
}

impl Db {
    pub fn init(path: &Path) -> color_eyre::Result<Self> {
        if let Some(parent) = path.parent() {
            crate::create_dir_err!(std::fs::create_dir_all(parent), parent)?;
        }

        let conn = Connection::open(path)
            .wrap_err_with(|| format!("Failed to open connection at `{}`", path.display()))?;

        conn.execute(
            &format!(
                "CREATE TABLE IF NOT EXISTS {} (
                target_path TEXT PRIMARY KEY,
                deploy_type TEXT NOT NULL,
                source_path TEXT NOT NULL,
                hash TEXT,
                symlink_target TEXT
            )",
                TABLE_NAME
            ),
            [],
        )
        .wrap_err_with(|| format!("Failed to create table '{TABLE_NAME}'"))?;

        Ok(Self { conn })
    }

    pub fn insert_or_update(&self, entry: &DbEntry) -> color_eyre::Result<()> {
        let hash_str = entry.hash.map(|h| format!("{:x}", h));
        let symlink_target_str = entry
            .symlink_target
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned());

        self.conn
            .execute(
                &format!("INSERT OR REPLACE INTO {} (target_path, deploy_type, source_path, hash, symlink_target) VALUES (?1, ?2, ?3, ?4, ?5)", TABLE_NAME),
                params![
                    entry.target_path.to_string_lossy(),
                    entry.deploy_type.to_string(),
                    entry.source_path.to_string_lossy(),
                    hash_str,
                    symlink_target_str,
                ],
            )
            .wrap_err_with(|| format!("Failed to insert/update entry for '{}'", entry.target_path.display()))?;

        Ok(())
    }

    pub fn delete_entry(&self, target: &Path) -> color_eyre::Result<()> {
        self.conn
            .execute(
                &format!("DELETE FROM {} WHERE target_path = ?1", TABLE_NAME),
                params![target.to_string_lossy()],
            )
            .wrap_err_with(|| format!("Failed to delete entry for '{}'", target.display()))?;
        Ok(())
    }

    pub fn delete_entry_with_prefix(&self, target: &Path) -> color_eyre::Result<()> {
        self.conn
            .execute(
                &format!("DELETE FROM {} WHERE target_path like ?1", TABLE_NAME),
                // TODO escape meta char
                params![target.to_string_lossy() + "%"],
            )
            .wrap_err_with(|| format!("Failed to delete entries with prefix '{}'", target.display()))?;
        Ok(())
    }

    pub fn delete_table(&self) -> color_eyre::Result<()> {
        self.conn
            .execute(&format!("DROP TABLE IF EXISTS {}", TABLE_NAME), [])
            .wrap_err_with(|| format!("Failed to delete table '{TABLE_NAME}'"))?;
        Ok(())
    }

    pub fn get_entry(&self, target: &Path) -> color_eyre::Result<Option<DbEntry>> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT target_path, deploy_type, source_path, hash, symlink_target FROM {} WHERE target_path = ?1",
                TABLE_NAME
            ))
            .wrap_err_with(|| format!("Failed to prepare statement for querying '{}'", target.display()))?;

        stmt.query_row(params![target.to_string_lossy()], row_to_entry)
            .optional()
            .wrap_err_with(|| format!("Failed to query entry for '{}'", target.display()))
    }

    pub fn get_all_entries(&self) -> color_eyre::Result<Vec<DbEntry>> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT target_path, deploy_type, source_path, hash, symlink_target FROM {}",
                TABLE_NAME
            ))
            .wrap_err("Failed to prepare statement for listing entries")?;

        let entries = stmt
            .query_map([], row_to_entry)
            .optional()
            .wrap_err("Failed to query entries from database")?;

        let mut result = Vec::new();
        if let Some(entries) = entries {
            for entry in entries {
                result.push(entry.wrap_err("Failed to read database entry")?);
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_db_init() {
        let temp_dir = tempdir().unwrap();
        let path = &temp_dir.path().join("db");
        let _db1 = Db::init(path).unwrap();
        let _db2 = Db::init(path).unwrap();
        let _db3 = Db::init(path).unwrap();
    }

    #[test]
    fn test_db_insert() {
        let temp_dir = tempdir().unwrap();
        let db = Db::init(&temp_dir.path().join("db")).unwrap();

        let entry = DbEntry::default();

        assert!(db.get_entry(&entry.target_path).unwrap().is_none());
        db.insert_or_update(&entry).unwrap();
        assert_eq!(
            db.get_entry(&entry.target_path).unwrap(),
            Some(DbEntry::default())
        );
    }

    #[test]
    fn test_db_multiple_insert() {
        let temp_dir = tempdir().unwrap();
        let db = Db::init(&temp_dir.path().join("db")).unwrap();

        for p in 'a'..='z' {
            db.insert_or_update(&DbEntry {
                target_path: PathBuf::from(p.to_string()),
                ..Default::default()
            })
            .unwrap();
        }

        assert_eq!(db.get_all_entries().unwrap().len(), 26);
    }

    #[test]
    fn test_db_update() {
        let temp_dir = tempdir().unwrap();
        let db = Db::init(&temp_dir.path().join("db")).unwrap();

        let entry = DbEntry::default();
        let new_entry = DbEntry {
            target_path: entry.target_path.clone(),
            hash: Some(1),
            ..Default::default()
        };

        db.insert_or_update(&entry).unwrap();
        db.insert_or_update(&new_entry).unwrap();
        assert_eq!(db.get_entry(&entry.target_path).unwrap(), Some(new_entry));
        assert_eq!(db.get_all_entries().iter().len(), 1);
    }

    #[test]
    fn test_db_delete() {
        let temp_dir = tempdir().unwrap();
        let db = Db::init(&temp_dir.path().join("db")).unwrap();

        let entry = DbEntry::default();

        db.insert_or_update(&entry).unwrap();
        assert!(db.get_entry(&entry.target_path).unwrap().is_some());
        db.delete_entry(&entry.target_path).unwrap();
        assert!(db.get_entry(&entry.target_path).unwrap().is_none());
    }

    #[test]
    fn test_db_delete_with_prefix() {
        let temp_dir = tempdir().unwrap();
        let db = Db::init(&temp_dir.path().join("db")).unwrap();

        db.insert_or_update(&DbEntry {
            target_path: PathBuf::from("/a/b"),
            ..Default::default()
        })
        .unwrap();
        db.insert_or_update(&DbEntry {
            target_path: PathBuf::from("/a/c"),
            ..Default::default()
        })
        .unwrap();
        db.insert_or_update(&DbEntry {
            target_path: PathBuf::from("/b/a"),
            ..Default::default()
        })
        .unwrap();

        db.delete_entry_with_prefix(Path::new("/ab")).unwrap();
        assert_eq!(db.get_all_entries().unwrap().len(), 3);
        db.delete_entry_with_prefix(Path::new("/a")).unwrap();
        assert_eq!(db.get_all_entries().unwrap().len(), 1);
    }

    #[test]
    fn test_db_delete_table() {
        let temp_dir = tempdir().unwrap();
        let db = Db::init(&temp_dir.path().join("db")).unwrap();
        db.delete_table().unwrap();
    }
}
