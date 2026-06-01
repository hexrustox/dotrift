use std::path::{Path, PathBuf};

use crate::{
    command::{to_absolute_path, util::is_managed_entry},
    db::Db,
    output,
};
use miette::Result;

pub fn list(file: Option<PathBuf>, db_path: &Path) -> Result<()> {
    let file = file.map(|p| to_absolute_path(&p)).transpose()?;
    let db = Db::init(db_path)?;

    if let Some(target) = file {
        match db.get_entry(&target)? {
            Some(entry) if is_managed_entry(&entry, &target, None) => {
                output::print_managed(&target, &entry.source_path, entry.deploy_type);
            }
            _ => {
                output::print_unmanaged(&target);
            }
        }
    } else {
        let entries = db.get_all_entries()?;
        for entry in &entries {
            output!(
                "{}",
                output::portal_str(&entry.target_path, &entry.source_path, entry.deploy_type)
            );
        }
        output::print_summary(format_args!(
            "{} {}",
            entries.len(),
            if entries.len() == 1 {
                "entry"
            } else {
                "entries"
            },
        ));
    }

    Ok(())
}

pub fn clear(file: Option<PathBuf>, db_path: &Path) -> Result<()> {
    let file = file.map(|p| to_absolute_path(&p)).transpose()?;
    let db = Db::init(db_path)?;

    if let Some(target) = file {
        db.delete_entry(&target)?;
    } else {
        let count = db.get_all_entries()?.len();
        db.delete_table()?;
        output::print_ok(format_args!("cleared {} entries", count));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{cli::ApplyFlags, command::util::tests::setup_test};
    use tempfile::TempDir;

    use super::*;

    fn setup_status() -> (TempDir, PathBuf, PathBuf, PathBuf) {
        let (temp_dir, source_dir, target_dir) = setup_test(r#""" = """#, "", "", true);
        let db_path = temp_dir.path().join("db");
        crate::command::apply::run(
            crate::cli::GlobalFlags::new(
                Some(source_dir.to_path_buf()),
                Some(target_dir.to_path_buf()),
                None,
            ),
            &db_path,
            ApplyFlags {
                dry_run: false,
                clean_up: false,
                prune_empty_dirs: false,
            },
        )
        .unwrap();
        (temp_dir, source_dir, target_dir, db_path)
    }

    #[test]
    fn test_status_list_single() {
        let (tmp, _source_dir, target_dir, db_path) = setup_status();
        list(Some(target_dir.join("a.txt")), &db_path).unwrap();
        crate::command::util::assert_captured_output("status_list_single", tmp.path());
    }

    #[test]
    fn test_status_list_all() {
        let (tmp, _source_dir, _target_dir, db_path) = setup_status();
        list(None, &db_path).unwrap();
        crate::command::util::assert_captured_output("status_list_all", tmp.path());
    }

    #[test]
    fn test_status_clear_single() {
        let (_tmp, _source_dir, target_dir, db_path) = setup_status();
        clear(Some(target_dir.join("a.txt")), &db_path).unwrap();
        let db = Db::init(&db_path).unwrap();
        assert!(db.get_entry(&target_dir.join("a.txt")).unwrap().is_none());
        assert!(db.get_entry(&target_dir.join("b.txt")).unwrap().is_some());
    }

    #[test]
    fn test_status_clear_all() {
        let (tmp, _source_dir, _target_dir, db_path) = setup_status();
        clear(None, &db_path).unwrap();
        crate::command::util::assert_captured_output("status_clear_all", tmp.path());
    }
}
