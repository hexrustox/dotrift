use crossterm::style::Color;
use miette::Result;
use tui::apply_color;

use crate::{color_enabled, managed, prettify_path, println_capture, state::StateDatabase};

pub fn run() -> Result<()> {
    let Some(database) = StateDatabase::open_read_only()? else {
        return Ok(());
    };
    let mut records = database.managed_paths()?;
    // TODO add sort options
    records.sort_by(|left, right| left.target_path.cmp(&right.target_path));

    for record in records {
        let managed = managed::is_managed(&record)?;
        let (verdict, color) = if managed {
            ("managed", Color::Green)
        } else {
            ("unmanaged", Color::Red)
        };
        println_capture!(
            "{:<10} {:<8} {} <- {}",
            apply_color(verdict, color, color_enabled!()),
            record.kind.as_str(),
            prettify_path(&record.target_path).display(),
            prettify_path(&record.source_path).display()
        );
    }
    Ok(())
}
