use crossterm::style::Color;
use miette::Result;
use tui::apply_color;

use crate::{color_enabled, managed, prettify_path, println_capture, state::StateDatabase};

pub fn run() -> Result<()> {
    let Some(database) = StateDatabase::open_read_only()? else {
        return Ok(());
    };
    let mut records = database.managed_paths()?;
    records.sort_by(|left, right| left.target_path.cmp(&right.target_path));

    for record in records {
        let managed = managed::is_managed(&record)?;
        let (verdict, color) = if managed {
            ("managed", Color::Green)
        } else {
            ("unmanaged", Color::Red)
        };
        // Pad the plain verdict word first: a colored word carries escape
        // bytes that defeat the field width and collapse the columns.
        println_capture!(
            "{} {:<8} {} <- {}",
            apply_color(format!("{verdict:<10}"), color, color_enabled!()),
            record.kind.as_str(),
            prettify_path(&record.target_path).display(),
            prettify_path(&record.source_path).display()
        );
    }
    Ok(())
}
