use crate::{platform::prettify_path, report::Outcome};

pub fn run(env: &crate::platform::Environment, color: bool) -> miette::Result<()> {
    let report = crate::report::Reporter::always(color);
    let Some(database) = crate::state::StateDatabase::open_read_only(env)? else {
        return Ok(());
    };
    let mut records = database.managed_paths()?;
    records.sort_by(|left, right| left.target_path.cmp(&right.target_path));

    for record in records {
        let managed = crate::state::is_managed(&record)?;
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
