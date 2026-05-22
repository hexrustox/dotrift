use std::{io, path::Path};

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
};

use super::{PagerMode, file_viewer::FileViewer, header};

pub struct View {
    viewer: FileViewer,
    header: String,
    viewport_h: usize,
}

impl View {
    pub fn new(path: &Path) -> io::Result<Self> {
        let viewer = FileViewer::new(path)?;
        Ok(Self {
            viewer,
            header: format!("File {} blocks directory creation", path.display()),
            viewport_h: 0,
        })
    }
}

impl PagerMode for View {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let total = self.viewer.lines_count();
        let max_vp = area.height.saturating_sub(1) as usize;
        let footer_h = if total > max_vp { 1u16 } else { 0 };

        let [header_area, content_area, footer_area] = {
            let c = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Min(0),
                    Constraint::Length(footer_h),
                ])
                .split(area);
            [c[0], c[1], c[2]]
        };
        self.viewport_h = content_area.height as usize;

        header::render(frame, header_area, &self.header);
        self.viewer.render(frame, content_area);
        if footer_h > 0 {
            let vp_h = content_area.height as usize;
            let max_pos = total.saturating_sub(vp_h) + 1;
            header::render(
                frame,
                footer_area,
                &format!("{}/{}", self.viewer.scroll_pos() + 1, max_pos),
            );
        }
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
