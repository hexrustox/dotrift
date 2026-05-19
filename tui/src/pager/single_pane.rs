use std::{io, path::Path};

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
};

use super::{PagerMode, file_viewer::FileViewer, header};

pub struct SinglePane {
    viewer: FileViewer,
    header_text: String,
}

impl SinglePane {
    pub fn new(path: &Path) -> io::Result<Self> {
        let viewer = FileViewer::new(path)?;
        Ok(Self {
            viewer,
            header_text: path.display().to_string(),
        })
    }
}

impl PagerMode for SinglePane {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);
        header::render(frame, chunks[0], &self.header_text);
        self.viewer
            .render(frame, chunks[1])
            .expect("Failed to render file content");
    }

    fn scroll_up(&mut self, n: usize) {
        self.viewer.scroll_up(n);
    }

    fn scroll_down(&mut self, n: usize, viewport_h: usize) {
        self.viewer.scroll_down(n, viewport_h);
    }

    fn scroll_to_top(&mut self) {
        self.viewer.scroll_to_top();
    }

    fn scroll_to_bottom(&mut self, viewport_h: usize) {
        self.viewer.scroll_to_bottom(viewport_h);
    }
}
