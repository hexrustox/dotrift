use ratatui::{
    Frame,
    layout::Rect,
    style::{Style, Stylize},
    text::Line,
};

pub fn render(frame: &mut Frame, area: Rect, text: &str) {
    let header = Line::from(text).style(Style::new().reversed());
    frame.render_widget(header, area);
}
