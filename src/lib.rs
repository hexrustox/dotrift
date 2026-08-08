pub mod cli;
pub mod managed;
pub mod state;

use std::path::Path;

pub fn status_lines(database: &state::StateDatabase) -> Result<Vec<String>, state::StateError> {
    let mut records = database.managed_paths()?;
    records.sort_by(|left, right| left.target_path.cmp(&right.target_path));

    records
        .into_iter()
        .map(|record| {
            let managed = managed::is_managed(&record)?;
            let verdict = if managed { "managed" } else { "unmanaged" };
            Ok(format!(
                "[{verdict}]   {:<9} {}",
                record.kind,
                record.target_path.display()
            ))
        })
        .collect()
}

pub fn ensure_absolute(path: &Path) -> std::io::Result<std::path::PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::*;
    use crate::state::{Kind, StateRecord};

    #[test]
    fn status_lines_are_sorted_by_target_path_and_report_recorded_kind() {
        let directory = tempdir().unwrap();
        let database = state::StateDatabase::open(directory.path()).unwrap();
        let first = directory.path().join("z");
        let second = directory.path().join("a");
        std::fs::write(&first, b"changed").unwrap();
        std::os::unix::fs::symlink("source", &second).unwrap();
        database
            .put(&StateRecord {
                target_path: first,
                source_path: PathBuf::from("source-z"),
                kind: Kind::File,
                link_target: None,
                content_hash: Some("0000000000000000".into()),
            })
            .unwrap();
        database
            .put(&StateRecord {
                target_path: second,
                source_path: PathBuf::from("source-a"),
                kind: Kind::Symlink,
                link_target: Some(PathBuf::from("source")),
                content_hash: None,
            })
            .unwrap();

        let lines = status_lines(&database).unwrap();
        assert!(lines[0].contains("[managed]"));
        assert!(lines[0].contains("symlink"));
        assert!(lines[1].contains("[unmanaged]"));
        assert!(lines[1].contains("file"));
    }
}
