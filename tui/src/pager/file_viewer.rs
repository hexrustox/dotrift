use std::{
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::Path,
};

use ratatui::{Frame, layout::Rect, widgets::Paragraph};

pub struct FileViewer {
    file: File,
    offsets: Vec<u64>,
    scroll: usize,
    buf: Vec<u8>,
}

impl FileViewer {
    pub fn new(path: &Path) -> std::io::Result<Self> {
        let mut file = File::open(path)?;
        let mut reader = BufReader::new(&mut file);
        let mut offsets = vec![0u64];
        let mut buf = Vec::new();

        loop {
            let bytes = reader.read_until(b'\n', &mut buf)?;
            if bytes == 0 {
                break;
            }
            offsets.push(offsets.last().unwrap() + bytes as u64);
            buf.clear();
        }

        file.seek(SeekFrom::Start(0))?;

        Ok(Self {
            file,
            offsets,
            scroll: 0,
            buf: Vec::new(),
        })
    }

    pub fn lines_count(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    pub fn scroll_down(&mut self, n: usize, viewport_height: usize) {
        let max = self.lines_count().saturating_sub(viewport_height);
        self.scroll = (self.scroll + n).min(max);
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll = 0;
    }

    pub fn scroll_to_bottom(&mut self, viewport_height: usize) {
        self.scroll = self.lines_count().saturating_sub(viewport_height);
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let line_count = self.lines_count();
        if line_count == 0 {
            frame.render_widget(Paragraph::new(""), area);
            return;
        }

        let visible_height = area.height as usize;
        let max_scroll = line_count.saturating_sub(visible_height);
        self.scroll = self.scroll.min(max_scroll);
        let start = self.scroll;
        let end = (start + visible_height).min(line_count);

        let _ = self.file.seek(SeekFrom::Start(self.offsets[start]));

        self.buf.clear();
        let mut reader = BufReader::new(&mut self.file);

        for _ in start..end {
            let bytes = reader.read_until(b'\n', &mut self.buf);
            if bytes.as_ref().is_ok_and(|b| *b == 0) || bytes.is_err() {
                break;
            }
        }

        let content = String::from_utf8_lossy(&self.buf);
        frame.render_widget(Paragraph::new(content), area);
    }
}
