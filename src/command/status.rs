use std::path::{Path, PathBuf};

use crate::{
    command::util::{is_managed, print_portal},
    db::Db,
};

pub fn run(file: Option<PathBuf>, db_path: &Path) -> color_eyre::Result<()> {
    let db = Db::init(db_path)?;

    if let Some(target) = file {
        match db.get_entry(&target)? {
            Some(entry) if is_managed(&entry.target_path, &db) => {
                println!(
                    "[MANAGED] {}",
                    print_portal(&target, &entry.reference, entry.action_type)
                );
            }
            _ => {
                println!("[UNMANAGED] {}", target.display());
            }
        }
    } else {
        for entry in db.get_all_entries()? {
            println!(
                "{}",
                print_portal(&entry.target_path, &entry.reference, entry.action_type)
            );
        }
    }

    Ok(())
}
