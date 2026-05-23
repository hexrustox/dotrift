use std::path::Path;

use ratatui::{
    Frame,
    layout::Rect,
    style::{Style, Stylize},
    text::Line,
};

use super::compact_path;

pub fn render(frame: &mut Frame, area: Rect, left: &str, center: &Path) {
    let width = area.width as usize;

    let right = "h help";
    let center = compact_path(center, width / 3);

    let mid = width / 2;
    let center_start = mid.saturating_sub(center.len() / 2);
    let left_pad = center_start.saturating_sub(left.len());
    let right_pad = width.saturating_sub(center_start + center.len() + right.len());
    let line = format!(
        "{}{:l$}{}{:r$}{}",
        left,
        "",
        center,
        "",
        right,
        l = left_pad,
        r = right_pad,
    );

    frame.render_widget(Line::from(line).style(Style::new().reversed()), area);
}
