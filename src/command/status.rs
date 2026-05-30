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
            println!(
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
        output::print_ok(format_args!("Cleared {} entries", count));
    }

    Ok(())
}
