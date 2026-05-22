use ratatui::{
    Frame,
    layout::Rect,
    style::{Style, Stylize},
    text::Line,
};

pub fn render(frame: &mut Frame, area: Rect, left: &str, right: &str) {
    let width = area.width as usize;
    let line = if right.is_empty() {
        Line::from(left.to_string())
    } else {
        let left_width = width.saturating_sub(right.len());
        Line::from(format!("{0:<1$}{2}", left, left_width, right))
    };
    frame.render_widget(line.style(Style::new().reversed()), area);
}
