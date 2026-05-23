use std::{
    io,
    path::{Path, PathBuf},
};

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
};

use super::{PagerMode, file_viewer::FileViewer, footer, scroll_status};

pub struct View {
    viewer: FileViewer,
    path: PathBuf,
    viewport_h: usize,
}

impl View {
    pub fn new(path: &Path) -> io::Result<Self> {
        let viewer = FileViewer::new(path)?;
        Ok(Self {
            viewer,
            path: path.to_path_buf(),
            viewport_h: 0,
        })
    }
}

impl PagerMode for View {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let [content_area, footer_area] = {
            let c = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(area);
            [c[0], c[1]]
        };
        self.viewport_h = content_area.height as usize;

        self.viewer.render(frame, content_area);

        let total = self.viewer.lines_count();
        let vp_h = content_area.height as usize;
        let status = if total > vp_h {
            scroll_status(self.viewer.scroll_pos(), total, vp_h)
        } else {
            String::new()
        };
        footer::render(frame, footer_area, &status, &self.path);
    }

    fn viewport_height(&self) -> usize {
        self.viewport_h
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
