use std::{
    fs::File,
    io::{self, BufRead, BufReader, Seek, SeekFrom},
    path::Path,
};

use memmap2::Mmap;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
};
use similar::{DiffOp, TextDiff};

use super::{PagerMode, Scroll, header, offsets_from_bytes};

#[derive(Clone, Copy, PartialEq)]
enum DiffTag {
    Equal,
    Delete,
    Insert,
    Change,
}

struct DiffPair {
    left_idx: Option<usize>,
    right_idx: Option<usize>,
    tag: DiffTag,
}

struct FileIndex {
    file: BufReader<File>,
    offsets: Vec<u64>,
}

impl FileIndex {
    fn new(file: BufReader<File>, offsets: Vec<u64>) -> Self {
        Self { file, offsets }
    }

    #[cfg(test)]
    fn from_reader(mut file: BufReader<File>) -> io::Result<Self> {
        use super::build_offsets;
        let (offsets, _) = build_offsets(&mut file)?;
        Ok(Self { file, offsets })
    }

    fn lines_count(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    fn read_line(&mut self, idx: usize, buf: &mut Vec<u8>) -> io::Result<()> {
        buf.clear();
        if idx >= self.lines_count() {
            return Ok(());
        }
        self.file.seek(SeekFrom::Start(self.offsets[idx]))?;
        self.file.read_until(b'\n', buf)?;
        strip_newline(buf);
        Ok(())
    }
}

fn strip_newline(buf: &mut Vec<u8>) {
    if buf.ends_with(b"\n") {
        buf.pop();
        if buf.ends_with(b"\r") {
            buf.pop();
        }
    }
}

pub struct SideBySide {
    pairs: Vec<DiffPair>,
    left: FileIndex,
    right: FileIndex,
    scroll: Scroll,
    buf: Vec<u8>,
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
                        left_idx: Some(old_index + i),
                        right_idx: Some(new_index + i),
                        tag: DiffTag::Equal,
                    });
                }
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => {
                for i in 0..old_len {
                    pairs.push(DiffPair {
                        left_idx: Some(old_index + i),
                        right_idx: None,
                        tag: DiffTag::Delete,
                    });
                }
            }
            DiffOp::Insert {
                new_index, new_len, ..
            } => {
                for i in 0..new_len {
                    pairs.push(DiffPair {
                        left_idx: None,
                        right_idx: Some(new_index + i),
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
                    let left_idx = (i < old_len).then_some(old_index + i);
                    let right_idx = (i < new_len).then_some(new_index + i);
                    let tag = match (i < old_len, i < new_len) {
                        (true, true) => DiffTag::Change,
                        (true, false) => DiffTag::Delete,
                        (false, true) => DiffTag::Insert,
                        (false, false) => unreachable!(),
                    };
                    pairs.push(DiffPair {
                        left_idx,
                        right_idx,
                        tag,
                    });
                }
            }
        }
    }

    pairs
}

impl SideBySide {
    pub fn new(path1: &Path, path2: &Path) -> io::Result<Self> {
        let left_file = File::open(path1)?;
        let right_file = File::open(path2)?;

        let left_map = unsafe { Mmap::map(&left_file)? };
        let right_map = unsafe { Mmap::map(&right_file)? };

        let left_str = std::str::from_utf8(&left_map)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let right_str = std::str::from_utf8(&right_map)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let pairs = compute_pairs_from_ops(left_str, right_str);

        let left_offsets = offsets_from_bytes(left_map.as_ref());
        let right_offsets = offsets_from_bytes(right_map.as_ref());

        drop(left_map);
        drop(right_map);

        let left = FileIndex::new(BufReader::new(left_file), left_offsets);
        let right = FileIndex::new(BufReader::new(right_file), right_offsets);

        Ok(Self {
            pairs,
            left,
            right,
            scroll: Scroll::new(),
            buf: Vec::new(),
        })
    }
}

fn styled_cell(prefix: char, content: String, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(prefix.to_string(), Style::default().fg(color)),
        Span::from(content),
    ])
}

fn read_content(fi: &mut FileIndex, idx: Option<usize>, buf: &mut Vec<u8>) -> String {
    match idx {
        Some(i) => {
            if fi.read_line(i, buf).is_err() {
                buf.clear();
            }
            String::from_utf8_lossy(buf).into_owned()
        }
        None => String::new(),
    }
}

fn pair_lines(
    pair: &DiffPair,
    left: &mut FileIndex,
    right: &mut FileIndex,
    buf: &mut Vec<u8>,
) -> (Line<'static>, Line<'static>) {
    let left_content = read_content(left, pair.left_idx, buf);
    let right_content = read_content(right, pair.right_idx, buf);

    let left_line = match (pair.tag, pair.left_idx) {
        (DiffTag::Delete | DiffTag::Change, Some(_)) => styled_cell('-', left_content, Color::Red),
        (DiffTag::Equal, Some(_)) => Line::from(format!(" {}", &left_content)),
        _ => Line::from(""),
    };

    let right_line = match (pair.tag, pair.right_idx) {
        (DiffTag::Insert | DiffTag::Change, Some(_)) => {
            styled_cell('+', right_content, Color::Green)
        }
        (DiffTag::Equal, Some(_)) => Line::from(format!(" {}", &right_content)),
        _ => Line::from(""),
    };

    (left_line, right_line)
}

impl PagerMode for SideBySide {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let [header_area, content_area] = {
            let c = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(0)])
                .split(area);
            [c[0], c[1]]
        };

        header::render(frame, header_area, "");

        let visible_h = content_area.height as usize;
        self.scroll.clamp(self.pairs.len(), visible_h);

        if self.pairs.is_empty() {
            return;
        }

        let left_width = content_area.width / 2;
        let right_width = content_area.width - left_width;
        let end = (self.scroll.get() + visible_h).min(self.pairs.len());

        for (row, pair_idx) in (self.scroll.get()..end).enumerate() {
            let pair = &self.pairs[pair_idx];
            let row_y = content_area.y + row as u16;

            let (left_line, right_line) =
                pair_lines(pair, &mut self.left, &mut self.right, &mut self.buf);

            let left_rect = Rect::new(content_area.x, row_y, left_width, 1);
            frame.render_widget(left_line, left_rect);

            let right_rect = Rect::new(content_area.x + left_width, row_y, right_width, 1);
            frame.render_widget(right_line, right_rect);
        }
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
    use super::*;
    use tempfile::NamedTempFile;
    use test_case::test_case;

    fn file_index_from_str(s: &str) -> (NamedTempFile, FileIndex) {
        let tmp = NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), s).unwrap();
        let fi = FileIndex::from_reader(BufReader::new(File::open(tmp.path()).unwrap())).unwrap();
        (tmp, fi)
    }

    fn spans_to_string(line: &Line) -> String {
        line.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<Vec<_>>()
            .concat()
    }

    #[test_case("a\nb\n",    "a\nb\n",    " a\n b",      " a\n b";      "equal")]
    #[test_case("a\nb\nc\n", "a\nc\n",    " a\n-b\n c", " a\n\n c";   "delete mid")]
    #[test_case("a\nc\n",    "a\nb\nc\n", " a\n\n c",   " a\n+b\n c"; "insert mid")]
    #[test_case("old\n",     "new\n",     "-old",           "+new";           "change")]
    #[test_case("a\nb\nc\n", "a\nx\nc\n", " a\n-b\n c", " a\n+x\n c"; "mixed change")]
    #[test_case("a\nb\n",    "a\n",       " a\n-b",       " a\n";         "solo delete")]
    #[test_case("a\n",       "a\nb\n",    " a\n",         " a\n+b";       "solo insert")]
    #[test_case("",          "",          "",                 "";                 "both empty")]
    #[test_case("",          "x\n",       "",               "+x";             "empty old")]
    #[test_case("x\n",       "",          "-x",             "";               "empty new")]
    fn diff(old: &str, new: &str, expected_left: &str, expected_right: &str) {
        let pairs = compute_pairs_from_ops(old, new);

        let (_tmp1, mut left) = file_index_from_str(old);
        let (_tmp2, mut right) = file_index_from_str(new);

        let mut buf = Vec::new();
        let mut actual_left = Vec::new();
        let mut actual_right = Vec::new();

        for pair in &pairs {
            let (l, r) = pair_lines(pair, &mut left, &mut right, &mut buf);
            actual_left.push(spans_to_string(&l));
            actual_right.push(spans_to_string(&r));
        }

        assert_eq!(actual_left.join("\n"), expected_left);
        assert_eq!(actual_right.join("\n"), expected_right);
    }
}
