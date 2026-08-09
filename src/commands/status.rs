use miette::Result;

use crate::state::StateDatabase;
use crate::{managed, println_capture};

pub fn run() -> Result<()> {
    let Some(database) = StateDatabase::open_read_only()? else {
        return Ok(());
    };
    let mut records = database.managed_paths()?;
    records.sort_by(|left, right| left.target_path.cmp(&right.target_path));

    for record in records {
        let managed = managed::is_managed(&record)?;
        let verdict = if managed { "managed" } else { "unmanaged" };
        println_capture!(
            "{:<9}   {:<9} {}",
            verdict,
            record.kind.as_str(),
            record.target_path.display()
        );
    }
    Ok(())
}
