use miette::Result;

use crate::managed;
use crate::state::StateDatabase;

pub fn run() -> Result<()> {
    let Some(database) = StateDatabase::open_read_only()? else {
        return Ok(());
    };
    let mut records = database.managed_paths()?;
    records.sort_by(|left, right| left.target_path.cmp(&right.target_path));

    for record in records {
        let managed = managed::is_managed(&record)?;
        let verdict = if managed { "managed" } else { "unmanaged" };
        println!(
            "[{verdict}]   {:<9} {}",
            record.kind,
            record.target_path.display()
        );
    }
    Ok(())
}
