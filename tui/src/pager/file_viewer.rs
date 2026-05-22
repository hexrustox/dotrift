use std::{
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::Path,
};

use ratatui::{Frame, layout::Rect, widgets::Paragraph};

use super::{Scroll, build_offsets};

pub struct FileViewer {
    file: BufReader<File>,
    offsets: Vec<u64>,
    scroll: Scroll,
    buf: Vec<u8>,
}

impl FileViewer {
    pub fn new(path: &Path) -> std::io::Result<Self> {
        let mut file = BufReader::new(File::open(path)?);
        let (offsets, buf) = build_offsets(&mut file)?;
        Ok(Self {
            file,
            offsets,
            scroll: Scroll::new(),
            buf,
        })
    }

    pub fn lines_count(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    pub fn scroll_pos(&self) -> usize {
        self.scroll.get()
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll.up(n);
    }

    pub fn scroll_down(&mut self, n: usize, viewport_height: usize) {
        self.scroll.down(n, self.lines_count(), viewport_height);
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll.top();
    }

    pub fn scroll_to_bottom(&mut self, viewport_height: usize) {
        self.scroll.bottom(self.lines_count(), viewport_height);
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let line_count = self.lines_count();
        if line_count == 0 {
            frame.render_widget(Paragraph::new(""), area);
            return;
        }

        let visible_h = area.height as usize;
        self.scroll.clamp(line_count, visible_h);
        let start = self.scroll.get();
        let end = (start + visible_h).min(line_count);

        let _ = self.file.seek(SeekFrom::Start(self.offsets[start]));

        self.buf.clear();
        for _ in start..end {
            if self.file.read_until(b'\n', &mut self.buf).is_err() {
                break;
            }
        }

        let content = String::from_utf8_lossy(&self.buf);
        frame.render_widget(Paragraph::new(content.as_ref()), area);
    }
}
