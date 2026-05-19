use std::{fs, io, path::Path};

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use similar::{ChangeTag, TextDiff};

use super::{PagerMode, header};

#[derive(Clone, Copy, PartialEq)]
enum DiffTag {
    Equal,
    Delete,
    Insert,
    Change,
}

struct DiffPair {
    left: Option<String>,
    right: Option<String>,
    tag: DiffTag,
}

pub struct SideBySide {
    pairs: Vec<DiffPair>,
    scroll: usize,
    header_left: String,
    header_right: String,
}

impl SideBySide {
    pub fn new(source: &Path, target: &Path) -> io::Result<Self> {
        let left_content = fs::read_to_string(source)?;
        let right_content = fs::read_to_string(target)?;
        let pairs = compute_diff(&left_content, &right_content);

        let header_left = source.display().to_string();
        let header_right = target.display().to_string();

        Ok(Self {
            pairs,
            scroll: 0,
            header_left,
            header_right,
        })
    }
}

fn compute_diff(old: &str, new: &str) -> Vec<DiffPair> {
    let diff = TextDiff::from_lines(old, new);

    let changes: Vec<(ChangeTag, String)> = diff
        .iter_all_changes()
        .map(|c| {
            let mut line = c.value().to_string();
            if line.ends_with('\n') {
                line.pop();
                if line.ends_with('\r') {
                    line.pop();
                }
            }
            (c.tag(), line)
        })
        .collect();

    let mut pairs = Vec::new();
    let mut i = 0;

    while i < changes.len() {
        let (tag, value) = &changes[i];
        let line = value.clone();

        match *tag {
            ChangeTag::Delete if i + 1 < changes.len() && changes[i + 1].0 == ChangeTag::Insert => {
                let new_line = changes[i + 1].1.clone();
                pairs.push(DiffPair {
                    left: Some(line),
                    right: Some(new_line),
                    tag: DiffTag::Change,
                });
                i += 2;
            }
            ChangeTag::Equal => {
                pairs.push(DiffPair {
                    left: Some(line.clone()),
                    right: Some(line),
                    tag: DiffTag::Equal,
                });
                i += 1;
            }
            ChangeTag::Delete => {
                pairs.push(DiffPair {
                    left: Some(line),
                    right: None,
                    tag: DiffTag::Delete,
                });
                i += 1;
            }
            ChangeTag::Insert => {
                pairs.push(DiffPair {
                    left: None,
                    right: Some(line),
                    tag: DiffTag::Insert,
                });
                i += 1;
            }
        }
    }

    pairs
}

fn styled_cell(prefix: char, content: &str, color: Color) -> Line<'_> {
    Line::from(vec![
        Span::styled(prefix.to_string(), Style::default().fg(color)),
        Span::from(content.to_string()),
    ])
}

fn pair_lines(pair: &DiffPair, left_w: usize, right_w: usize) -> (Line<'_>, Line<'_>) {
    let left = match (pair.tag, &pair.left) {
        (DiffTag::Delete | DiffTag::Change, Some(s)) => {
            styled_cell('-', truncate(s, left_w.saturating_sub(1)), Color::Red)
        }
        (DiffTag::Equal, Some(s)) => {
            Line::from(format!(" {}", truncate(s, left_w.saturating_sub(1))))
        }
        _ => Line::from(""),
    };
    let right = match (pair.tag, &pair.right) {
        (DiffTag::Insert | DiffTag::Change, Some(s)) => {
            styled_cell('+', truncate(s, right_w.saturating_sub(1)), Color::Green)
        }
        (DiffTag::Equal, Some(s)) => {
            Line::from(format!(" {}", truncate(s, right_w.saturating_sub(1))))
        }
        _ => Line::from(""),
    };
    (left, right)
}

impl PagerMode for SideBySide {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);
        let header_area = chunks[0];
        let content_area = chunks[1];

        let header_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
            .split(header_area);
        header::render(frame, header_chunks[0], &self.header_left);
        header::render(frame, header_chunks[1], &self.header_right);

        let visible_h = content_area.height as usize;
        let max_scroll = self.pairs.len().saturating_sub(visible_h);
        self.scroll = self.scroll.min(max_scroll);

        if self.pairs.is_empty() {
            return;
        }

        let left_width = content_area.width / 2;
        let right_width = content_area.width - left_width;
        let end = (self.scroll + visible_h).min(self.pairs.len());

        for (row, pair_idx) in (self.scroll..end).enumerate() {
            let pair = &self.pairs[pair_idx];
            let row_y = content_area.y + row as u16;

            let (left_line, right_line) =
                pair_lines(pair, left_width as usize, right_width as usize);
            let left_rect = Rect::new(content_area.x, row_y, left_width, 1);
            frame.render_widget(Paragraph::new(left_line), left_rect);

            let right_rect = Rect::new(content_area.x + left_width, row_y, right_width, 1);
            frame.render_widget(Paragraph::new(right_line), right_rect);
        }
    }

    fn scroll_up(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    fn scroll_down(&mut self, n: usize, viewport_h: usize) {
        let max = self.pairs.len().saturating_sub(viewport_h);
        self.scroll = (self.scroll + n).min(max);
    }

    fn scroll_to_top(&mut self) {
        self.scroll = 0;
    }

    fn scroll_to_bottom(&mut self, viewport_h: usize) {
        self.scroll = self.pairs.len().saturating_sub(viewport_h);
    }
}

fn truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    fn columns_lines(
        pairs: &[DiffPair],
        left_w: usize,
        right_w: usize,
    ) -> (Vec<Line<'_>>, Vec<Line<'_>>) {
        let mut left = Vec::new();
        let mut right = Vec::new();
        for pair in pairs {
            let (l, r) = pair_lines(pair, left_w, right_w);
            left.push(l);
            right.push(r);
        }
        (left, right)
    }

    fn line_from_text(text: &str) -> Line<'_> {
        if text.is_empty() {
            return Line::from("");
        }
        let first = text.as_bytes()[0];
        match first {
            b'-' => styled_cell('-', &text[1..], Color::Red),
            b'+' => styled_cell('+', &text[1..], Color::Green),
            _ => Line::from(text.to_string()),
        }
    }

    #[test_case("a\nb\n", "a\nb\n", &[" a", " b"], &[" a", " b"]; "equal")]
    #[test_case("a\nb\nc\n", "a\nc\n", &[" a", "-b", " c"], &[" a", "", " c"]; "delete mid")]
    #[test_case("a\nc\n", "a\nb\nc\n", &[" a", "", " c"], &[" a", "+b", " c"]; "insert mid")]
    #[test_case("old\n", "new\n", &["-old"], &["+new"]; "change")]
    #[test_case("a\nb\nc\n", "a\nx\nc\n", &[" a", "-b", " c"], &[" a", "+x", " c"]; "mixed change")]
    #[test_case("a\nb\n", "a\n", &[" a", "-b"], &[" a", ""]; "solo delete")]
    #[test_case("a\n", "a\nb\n", &[" a", ""], &[" a", "+b"]; "solo insert")]
    #[test_case("", "", &[], &[]; "both empty")]
    #[test_case("", "x\n", &[""], &["+x"]; "empty old")]
    #[test_case("x\n", "", &["-x"], &[""]; "empty new")]
    fn diff(old: &str, new: &str, expected_left: &[&str], expected_right: &[&str]) {
        let pairs = compute_diff(old, new);
        let (left_lines, right_lines) = columns_lines(&pairs, 80, 80);

        let exp_left: Vec<Line> = expected_left.iter().map(|s| line_from_text(s)).collect();
        let exp_right: Vec<Line> = expected_right.iter().map(|s| line_from_text(s)).collect();

        assert_eq!(left_lines, exp_left);
        assert_eq!(right_lines, exp_right);
    }
}
