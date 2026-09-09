use miette::Result;

use crate::{
    environment::Environment,
    fingerprint, prettify_path,
    report::{Outcome, Reporter},
    state::StateDatabase,
};

pub fn run(env: &Environment, color: bool) -> Result<()> {
    let report = Reporter::always(color);
    let Some(database) = StateDatabase::open_read_only(env)? else {
        return Ok(());
    };
    let mut records = database.managed_paths()?;
    records.sort_by(|left, right| left.target_path.cmp(&right.target_path));

    for record in records {
        let managed = fingerprint::is_managed(&record)?;
        let (outcome, verdict) = if managed {
            (Outcome::Managed, "managed")
        } else {
            (Outcome::Unmanaged, "unmanaged")
        };
        // Pad the plain verdict word first: a colored word carries escape
        // bytes that defeat the field width and collapse the columns.
        report.line(format_args!(
            "{} {:<8} {} <- {}",
            report.paint(outcome, format!("{verdict:<10}")),
            record.kind.as_str(),
            prettify_path(&record.target_path).display(),
            prettify_path(&record.source_path).display()
        ));
    }
    Ok(())
}
