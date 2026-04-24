use std::path::{Path, PathBuf};

use crate::{
    command::{
        to_absolute_path,
        util::{is_managed, print_portal},
    },
    db::Db,
};
use color_eyre::Result;

pub fn list(file: Option<PathBuf>, db_path: &Path) -> Result<()> {
    let file = file.map(|p| to_absolute_path(&p)).transpose()?;
    let db = Db::init(db_path)?;

    if let Some(target) = file {
        match db.get_entry(&target)? {
            Some(entry) if is_managed(&entry.target_path, &db, None) => {
                eprintln!(
                    "[MANAGED] {}",
                    print_portal(&target, &entry.source_path, entry.deploy_type)
                );
            }
            _ => {
                eprintln!("[UNMANAGED] {}", target.display());
            }
        }
    } else {
        let entries = db.get_all_entries()?;
        for entry in &entries {
            eprintln!(
                "{}",
                print_portal(&entry.target_path, &entry.source_path, entry.deploy_type)
            );
        }
        eprintln!(
            "Total: {} file{}",
            entries.len(),
            if entries.len() > 1 { "s" } else { "" }
        )
    }

    Ok(())
}

pub fn clear(file: Option<PathBuf>, db_path: &Path) -> Result<()> {
    let file = file.map(|p| to_absolute_path(&p)).transpose()?;
    let db = Db::init(db_path)?;

    if let Some(target) = file {
        db.delete_entry(&target)?;
    } else {
        db.delete_table()?;
        eprintln!("Cleared all entries");
    }

    Ok(())
}
