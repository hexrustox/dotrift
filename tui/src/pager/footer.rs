use ratatui::{
    Frame,
    layout::Rect,
    style::{Style, Stylize},
    text::Line,
};

pub fn render(frame: &mut Frame, area: Rect, left: &str, center: &str, right: &str) {
    let width = area.width as usize;

    let left = truncate(left, width);
    let center = truncate(center, width);
    let right = truncate(right, width);

    let line = if left.len() + center.len() + right.len() + 4 <= width {
        let mid = width / 2;
        let center_start = mid.saturating_sub(center.len() / 2);
        let left_pad = center_start.saturating_sub(left.len());
        let right_pad = width.saturating_sub(center_start + center.len() + right.len());
        format!(
            "{}{:l$}{}{:r$}{}",
            left,
            "",
            center,
            "",
            right,
            l = left_pad,
            r = right_pad,
        )
    } else {
        format!(
            "{:<l$}{}",
            left,
            right,
            l = width.saturating_sub(right.len())
        )
    };

    frame.render_widget(Line::from(line).style(Style::new().reversed()), area);
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() > max { &s[..max] } else { s }
}
