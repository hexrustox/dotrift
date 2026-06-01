use std::{
    fs::File,
    io::{self, BufRead, BufReader, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use memmap2::Mmap;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use similar::{DiffOp, TextDiff};

use super::{PagerMode, Scroll, footer, offsets_from_bytes, scroll_status, splitter_char};

#[derive(Clone, Copy, PartialEq)]
enum DiffTag {
    Equal,
    Delete,
    Insert,
    Change,
}

struct DiffPair {
    old_idx: Option<usize>,
    new_idx: Option<usize>,
    tag: DiffTag,
}

impl DiffPair {
    fn pair_lines(
        &self,
        old_fi: &mut FileIndex,
        new_fi: &mut FileIndex,
        buf: &mut Vec<u8>,
    ) -> (Line<'static>, Line<'static>) {
        let old_content = old_fi.read_content(self.old_idx, buf);
        let new_content = new_fi.read_content(self.new_idx, buf);

        let old_line = match (self.tag, self.old_idx) {
            (DiffTag::Delete | DiffTag::Change, Some(_)) => {
                styled_cell('-', old_content, Color::Red)
            }
            (DiffTag::Equal, Some(_)) => Line::from(format!(" {}", &old_content)),
            _ => Line::from(""),
        };

        let new_line = match (self.tag, self.new_idx) {
            (DiffTag::Insert | DiffTag::Change, Some(_)) => {
                styled_cell('+', new_content, Color::Green)
            }
            (DiffTag::Equal, Some(_)) => Line::from(format!(" {}", &new_content)),
            _ => Line::from(""),
        };

        (old_line, new_line)
    }
}

struct FileIndex {
    file: BufReader<File>,
    offsets: Vec<u64>,
    read_seq: Option<usize>,
}

impl FileIndex {
    fn new(file: BufReader<File>, offsets: Vec<u64>) -> Self {
        Self {
            file,
            offsets,
            read_seq: None,
        }
    }

    fn lines_count(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    fn read_line(&mut self, idx: usize, buf: &mut Vec<u8>) -> io::Result<()> {
        buf.clear();
        if idx >= self.lines_count() {
            return Ok(());
        }
        if self.read_seq != Some(idx) {
            self.file.seek(SeekFrom::Start(self.offsets[idx]))?;
        }
        self.file.read_until(b'\n', buf)?;
        self.read_seq = Some(idx + 1);
        if buf.ends_with(b"\n") {
            buf.pop();
        }
        Ok(())
    }

    fn read_content(&mut self, idx: Option<usize>, buf: &mut Vec<u8>) -> String {
        match idx {
            Some(i) => {
                if self.read_line(i, buf).is_err() {
                    buf.clear();
                }
                String::from_utf8_lossy(buf).into_owned()
            }
            None => String::new(),
        }
    }
}

pub struct Diff {
    pairs: Vec<DiffPair>,
    old: FileIndex,
    new: FileIndex,
    scroll: Scroll,
    path: PathBuf,
    buf: Vec<u8>,
    added: usize,
    removed: usize,
    viewport_h: usize,
}

fn compute_pairs_from_ops(old: &str, new: &str) -> Vec<DiffPair> {
    let diff = TextDiff::from_lines(old, new);

    let mut pairs = Vec::new();

    for op in diff.ops() {
        match *op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                for i in 0..len {
                    pairs.push(DiffPair {
                        old_idx: Some(old_index + i),
                        new_idx: Some(new_index + i),
                        tag: DiffTag::Equal,
                    });
                }
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => {
                for i in 0..old_len {
                    pairs.push(DiffPair {
                        old_idx: Some(old_index + i),
                        new_idx: None,
                        tag: DiffTag::Delete,
                    });
                }
            }
            DiffOp::Insert {
                new_index, new_len, ..
            } => {
                for i in 0..new_len {
                    pairs.push(DiffPair {
                        old_idx: None,
                        new_idx: Some(new_index + i),
                        tag: DiffTag::Insert,
                    });
                }
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                let max = old_len.max(new_len);
                for i in 0..max {
                    let old_idx = (i < old_len).then_some(old_index + i);
                    let new_idx = (i < new_len).then_some(new_index + i);
                    let tag = match (i < old_len, i < new_len) {
                        (true, true) => DiffTag::Change,
                        (true, false) => DiffTag::Delete,
                        (false, true) => DiffTag::Insert,
                        (false, false) => unreachable!(),
                    };
                    pairs.push(DiffPair {
                        old_idx,
                        new_idx,
                        tag,
                    });
                }
            }
        }
    }

    pairs
}

impl Diff {
    pub fn new(old: &Path, new: &Path) -> io::Result<Self> {
        let old_file = File::open(old)?;
        let new_file = File::open(new)?;

        // SAFETY: This process has exclusive access to the file — opened read-only,
        // no concurrent writer modifies or truncates it while mapped.
        let old_map = unsafe { Mmap::map(&old_file)? };
        let new_map = unsafe { Mmap::map(&new_file)? };

        let old_str = std::str::from_utf8(&old_map)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let new_str = std::str::from_utf8(&new_map)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let pairs = compute_pairs_from_ops(old_str, new_str);

        let mut added = 0;
        let mut removed = 0;
        for pair in &pairs {
            match pair.tag {
                DiffTag::Insert => added += 1,
                DiffTag::Delete => removed += 1,
                DiffTag::Change => {
                    added += 1;
                    removed += 1;
                }
                _ => {}
            }
        }

        let old_offsets = offsets_from_bytes(old_map.as_ref());
        let new_offsets = offsets_from_bytes(new_map.as_ref());

        drop(old_map);
        drop(new_map);

        let old_fi = FileIndex::new(BufReader::new(old_file), old_offsets);
        let new_fi = FileIndex::new(BufReader::new(new_file), new_offsets);

        Ok(Self {
            pairs,
            old: old_fi,
            new: new_fi,
            scroll: Scroll::new(),
            path: old.to_path_buf(),
            buf: Vec::new(),
            added,
            removed,
            viewport_h: 0,
        })
    }
}

fn styled_cell(prefix: char, content: String, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(prefix.to_string(), Style::default().fg(color)),
        Span::from(content),
    ])
}

impl PagerMode for Diff {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let [content_area, footer_area] = {
            let c = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(area);
            [c[0], c[1]]
        };
        self.viewport_h = content_area.height as usize;

        let visible_h = content_area.height as usize;
        self.scroll.clamp(self.pairs.len(), visible_h);

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Ratio(1, 2),
                Constraint::Length(1),
                Constraint::Ratio(1, 2),
            ])
            .split(content_area);
        let old_area = columns[0];
        let splitter_area = columns[1];
        let new_area = columns[2];

        frame.render_widget(
            Paragraph::new(format!("{}\n", splitter_char()).repeat(splitter_area.height as usize)),
            splitter_area,
        );

        let end = (self.scroll.get() + visible_h).min(self.pairs.len());

        for (row, pair_idx) in (self.scroll.get()..end).enumerate() {
            let pair = &self.pairs[pair_idx];
            let row_y = content_area.y + row as u16;

            let (old_line, new_line) = pair.pair_lines(&mut self.old, &mut self.new, &mut self.buf);

            let old_rect = Rect::new(old_area.x, row_y, old_area.width, 1);
            frame.render_widget(old_line, old_rect);

            let new_rect = Rect::new(new_area.x, row_y, new_area.width, 1);
            frame.render_widget(new_line, new_rect);
        }

        footer::render(
            frame,
            footer_area,
            &format!(
                "{} (+{} -{})",
                scroll_status(self.scroll.get(), self.pairs.len(), visible_h),
                self.added,
                self.removed
            ),
            &self.path,
        );
    }

    fn viewport_height(&self) -> usize {
        self.viewport_h
    }

    fn scroll_up(&mut self, n: usize) {
        self.scroll.up(n);
    }

    fn scroll_down(&mut self, n: usize, viewport_h: usize) {
        self.scroll.down(n, self.pairs.len(), viewport_h);
    }

    fn scroll_to_top(&mut self) {
        self.scroll.top();
    }

    fn scroll_to_bottom(&mut self, viewport_h: usize) {
        self.scroll.bottom(self.pairs.len(), viewport_h);
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write};

    use super::*;
    use insta::assert_snapshot;
    use ratatui::{Terminal, backend::TestBackend};
    use tempfile::NamedTempFile;
    use test_case::test_case;

    #[test_case("diff_empty", "", ""; "empty")]
    #[test_case("diff_equal", "hello\nworld\n", "hello\nworld\n"; "equal")]
    #[test_case("diff_insert", "keep\n", "keep\nadded\n"; "insert")]
    #[test_case("diff_delete", "keep\nremove\n", "keep\n"; "delete")]
    #[test_case("diff_change", "old\n", "new\n"; "change")]
    #[test_case("diff_mixed", "a\nb\nc\n", "a\nx\nc\n"; "mixed")]
    fn test_render(snap_name: &str, old_str: &str, new_str: &str) {
        let old = NamedTempFile::new().unwrap();
        let new = NamedTempFile::new().unwrap();
        fs::write(old.path(), old_str).unwrap();
        fs::write(new.path(), new_str).unwrap();

        let mut diff = Diff::new(old.path(), new.path()).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(40, 5)).unwrap();
        terminal
            .draw(|f| {
                diff.render(f, f.area());
            })
            .unwrap();
        assert_snapshot!(snap_name, terminal.backend());
    }

    #[test]
    fn test_scroll() {
        let mut old = NamedTempFile::new().unwrap();
        let mut new = NamedTempFile::new().unwrap();
        for i in 1..=20 {
            writeln!(old, "line {i}").unwrap();
            writeln!(new, "line {i}").unwrap();
        }
        old.flush().unwrap();
        new.flush().unwrap();

        let mut diff = Diff::new(old.path(), new.path()).unwrap();
        let vp_h = 5;
        diff.scroll_down(5, vp_h);

        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        terminal
            .draw(|f| {
                diff.render(f, f.area());
            })
            .unwrap();
        assert_snapshot!("diff_scroll", terminal.backend());
    }
}
