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
        footer::render(
            frame,
            footer_area,
            &scroll_status(self.viewer.scroll_pos(), total, vp_h),
            &self.path,
        );
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

#[cfg(test)]
mod tests {
    use std::{fs, io::Write};

    use crate::pager::tests::{TERMINAL_WIDTH, assert_terminal};

    use super::*;

    use ratatui::{Terminal, backend::TestBackend};
    use tempfile::NamedTempFile;
    use test_case::test_case;

    #[test_case("view_empty", ""; "empty")]
    #[test_case("view_single_line", "hello\n"; "single_line")]
    #[test_case("view_multiple_fit", "line1\nline2\nline3\n"; "multiple_fit")]
    #[test_case("view_overflow_viewport", "a\nb\nc\nd\ne\nf\ng\nh\n"; "overflow_viewport")]
    fn test_render(snap_name: &str, content: &str) {
        let file = NamedTempFile::new().unwrap();
        fs::write(file.path(), content).unwrap();

        let mut view = View::new(file.path()).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(TERMINAL_WIDTH, 5)).unwrap();
        terminal
            .draw(|f| {
                view.render(f, f.area());
            })
            .unwrap();
        assert_terminal(file.path(), snap_name, terminal);
    }

    #[test]
    fn test_scroll() {
        let mut file = NamedTempFile::new().unwrap();
        for i in 1..=20 {
            writeln!(file, "line {i}").unwrap();
        }
        file.flush().unwrap();

        let mut view = View::new(file.path()).unwrap();
        let vp_h = 5;
        view.scroll_down(5, vp_h);

        let mut terminal = Terminal::new(TestBackend::new(TERMINAL_WIDTH, 10)).unwrap();
        terminal
            .draw(|f| {
                view.render(f, f.area());
            })
            .unwrap();
        assert_terminal(file.path(), "view_scroll", terminal);
    }
}
