use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, eyre};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    config::DeployType,
    error::{EyreError, IoError},
};

pub struct Db {
    conn: Connection,
}

#[derive(Default, Debug, PartialEq)]
pub struct DbEntry {
    pub target_path: PathBuf,
    pub deploy_type: DeployType,
    pub source_path: PathBuf,
    pub hash: Option<u64>,
}

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<DbEntry> {
    let target_str: String = row.get(0)?;
    let deploy_str: String = row.get(1)?;
    let source_str: String = row.get(2)?;
    let hash_str: Option<String> = row.get(3)?;

    let deploy_type = match deploy_str.as_str() {
        "symlink" => DeployType::Symlink,
        "copy" => DeployType::Copy,
        other => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                eyre!(r#"Invalid type: "{other}""#).into(),
            ));
        }
    };

    let hash = hash_str.and_then(|s| u64::from_str_radix(&s, 16).ok());

    Ok(DbEntry {
        target_path: PathBuf::from(target_str),
        deploy_type,
        source_path: PathBuf::from(source_str),
        hash,
    })
}

impl Db {
    pub fn init(path: &Path) -> color_eyre::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .create_dir_error(parent)
                .wrap_as_db_error()?;
        }

        let conn = Connection::open(path)
            .wrap_err_with(|| format!("Failed to open connection at `{}`", path.display()))
            .wrap_as_db_error()?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS entries (
                target_path TEXT PRIMARY KEY,
                deploy_type TEXT NOT NULL,
                source_path TEXT NOT NULL,
                hash TEXT
            )",
            [],
        )
        .wrap_err("Failed to create table")
        .wrap_as_db_error()?;

        Ok(Self { conn })
    }

    pub fn insert_or_update(&self, entry: &DbEntry) -> color_eyre::Result<()> {
        let hash_str = entry.hash.map(|h| format!("{:x}", h));

        self.conn
            .execute(
                "INSERT OR REPLACE INTO entries (target_path, deploy_type, source_path, hash) VALUES (?1, ?2, ?3, ?4)",
                params![
                    entry.target_path.to_string_lossy(),
                    entry.deploy_type.to_string(),
                    entry.source_path.to_string_lossy(),
                    hash_str,
                ],
            )
            .wrap_err("Failed to insert or update entry")
            .wrap_as_db_error()?;

        Ok(())
    }

    pub fn delete_entry(&self, target: &Path) -> color_eyre::Result<()> {
        self.conn
            .execute(
                "DELETE FROM entries WHERE target_path = ?1",
                params![target.to_string_lossy()],
            )
            .wrap_err("Failed to delete entry")
            .wrap_as_db_error()?;
        Ok(())
    }

    pub fn delete_entry_with_prefix(&self, target: &Path) -> color_eyre::Result<()> {
        self.conn
            .execute(
                "DELETE FROM entries WHERE target_path like ?1",
                params![target.to_string_lossy() + "%"],
            )
            .wrap_err("Failed to delete entries")
            .wrap_as_db_error()?;
        Ok(())
    }

    pub fn get_entry(&self, target: &Path) -> color_eyre::Result<Option<DbEntry>> {
        let mut stmt = self
        .conn
        .prepare("SELECT target_path, deploy_type, source_path, hash FROM entries WHERE target_path = ?1")
        .wrap_err("Failed to prepare statement").wrap_as_db_error()?;

        stmt.query_row(params![target.to_string_lossy()], row_to_entry)
            .optional()
            .wrap_err("Failed to query entry")
            .wrap_as_db_error()
    }

    pub fn get_all_entries(&self) -> color_eyre::Result<Vec<DbEntry>> {
        let mut stmt = self
            .conn
            .prepare("SELECT target_path, deploy_type, source_path, hash FROM entries")
            .wrap_err("Failed to prepare statement")
            .wrap_as_db_error()?;

        let entries = stmt
            .query_map([], row_to_entry)
            .optional()
            .wrap_err("Failed to query entries")
            .wrap_as_db_error()?;

        let mut result = Vec::new();
        if let Some(entries) = entries {
            for entry in entries {
                result.push(entry.wrap_err("Failed to query entry").wrap_as_db_error()?);
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
        assert!(db.get_entry(&entry.target_path).unwrap().is_some());
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

        db.delete_entry_with_prefix(Path::new("/a")).unwrap();
        assert_eq!(db.get_all_entries().unwrap().len(), 1);
    }
}
