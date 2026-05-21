use std::{fs, io, path::PathBuf};

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    widgets::{List, ListItem, ListState, Paragraph},
};

use super::{PagerMode, file_viewer::FileViewer, header};

struct DirEntry {
    name: String,
    is_dir: bool,
    symlink_target: Option<String>,
}

enum RightState {
    Browser {
        entries: Vec<DirEntry>,
        cursor: usize,
        path: PathBuf,
    },
    FileView {
        viewer: FileViewer,
        path: PathBuf,
    },
}

#[derive(Clone, Copy, PartialEq)]
enum Focus {
    Left,
    Right,
}

impl Focus {
    fn toggle(self) -> Self {
        match self {
            Focus::Left => Focus::Right,
            Focus::Right => Focus::Left,
        }
    }
}

pub struct Explorer {
    left: FileViewer,
    right_state: RightState,
    focus: Focus,
}

impl Explorer {
    pub fn new(source: &std::path::Path, target: &std::path::Path) -> io::Result<Self> {
        let left = FileViewer::new(source)?;
        let entries = read_entries(target)?;
        let right_state = RightState::Browser {
            entries,
            cursor: 0,
            path: target.to_path_buf(),
        };
        Ok(Self {
            left,
            right_state,
            focus: Focus::Left,
        })
    }
}

fn read_entries(path: &std::path::Path) -> io::Result<Vec<DirEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = ft.is_dir();
        let symlink_target = if ft.is_symlink() {
            fs::read_link(entry.path())
                .ok()
                .map(|p| p.display().to_string())
        } else {
            None
        };
        entries.push(DirEntry {
            name,
            is_dir,
            symlink_target,
        });
    }
    entries.sort_by(|a, b| {
        a.is_dir
            .cmp(&b.is_dir)
            .reverse()
            .then_with(|| a.name.cmp(&b.name))
    });
    if path.parent().is_some() {
        entries.insert(
            0,
            DirEntry {
                name: "..".to_string(),
                is_dir: true,
                symlink_target: None,
            },
        );
    }
    Ok(entries)
}

impl PagerMode for Explorer {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let [header_area, content_area] = {
            let c = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(0)])
                .split(area);
            [c[0], c[1]]
        };

        header::render(frame, header_area, "");

        let [browser_area, preview_area] = {
            let c = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)])
                .split(content_area);
            [c[0], c[1]]
        };

        match &mut self.right_state {
            RightState::Browser {
                entries,
                cursor,
                path: _,
            } => {
                if entries.is_empty() {
                    frame.render_widget(Paragraph::new("(empty)"), browser_area);
                } else {
                    let items: Vec<ListItem> = entries
                        .iter()
                        .map(|e| {
                            let display = match &e.symlink_target {
                                Some(t) => format!("{} -> {}", e.name, t),
                                None if e.is_dir => format!("{}/", e.name),
                                None => e.name.clone(),
                            };
                            ListItem::new(display)
                        })
                        .collect();

                    let list = List::new(items)
                        .highlight_symbol(if self.focus == Focus::Left {
                            "> "
                        } else {
                            "  "
                        })
                        .highlight_style(Style::default());

                    let mut state = ListState::default();
                    let max_cursor = entries.len().saturating_sub(1);
                    *cursor = (*cursor).min(max_cursor);
                    state.select(Some(*cursor));

                    frame.render_stateful_widget(list, browser_area, &mut state);
                }
            }
            RightState::FileView { viewer, .. } => {
                viewer.render(frame, browser_area);
            }
        }

        self.left.render(frame, preview_area);
    }

    fn scroll_up(&mut self, n: usize) {
        match self.focus {
            Focus::Left => match &mut self.right_state {
                RightState::Browser { cursor, .. } => {
                    *cursor = cursor.saturating_sub(n);
                }
                RightState::FileView { viewer, .. } => viewer.scroll_up(n),
            },
            Focus::Right => self.left.scroll_up(n),
        }
    }

    fn scroll_down(&mut self, n: usize, viewport_h: usize) {
        match self.focus {
            Focus::Left => match &mut self.right_state {
                RightState::Browser {
                    entries, cursor, ..
                } => {
                    let max = entries.len().saturating_sub(1);
                    *cursor = (*cursor + n).min(max);
                }
                RightState::FileView { viewer, .. } => viewer.scroll_down(n, viewport_h),
            },
            Focus::Right => self.left.scroll_down(n, viewport_h),
        }
    }

    fn scroll_to_top(&mut self) {
        match self.focus {
            Focus::Left => match &mut self.right_state {
                RightState::Browser { cursor, .. } => *cursor = 0,
                RightState::FileView { viewer, .. } => viewer.scroll_to_top(),
            },
            Focus::Right => self.left.scroll_to_top(),
        }
    }

    fn scroll_to_bottom(&mut self, viewport_h: usize) {
        match self.focus {
            Focus::Left => match &mut self.right_state {
                RightState::Browser {
                    entries, cursor, ..
                } => {
                    *cursor = entries.len().saturating_sub(1);
                }
                RightState::FileView { viewer, .. } => viewer.scroll_to_bottom(viewport_h),
            },
            Focus::Right => self.left.scroll_to_bottom(viewport_h),
        }
    }

    fn on_esc(&mut self) {
        if self.focus != Focus::Left {
            return;
        }
        match &self.right_state {
            RightState::Browser { path, .. } => {
                if let Some(parent) = path.parent()
                    && let Ok(entries) = read_entries(parent)
                {
                    self.right_state = RightState::Browser {
                        entries,
                        cursor: 0,
                        path: parent.to_path_buf(),
                    };
                }
            }
            RightState::FileView { path, .. } => {
                let parent = path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| path.clone());
                if let Ok(entries) = read_entries(&parent) {
                    self.right_state = RightState::Browser {
                        entries,
                        cursor: 0,
                        path: parent,
                    };
                }
            }
        }
    }

    fn on_enter(&mut self) {
        if self.focus != Focus::Left {
            return;
        }
        let (entry, path) = match &self.right_state {
            RightState::Browser {
                entries,
                cursor,
                path,
            } => {
                let Some(entry) = entries.get(*cursor) else {
                    return;
                };
                (entry, path.clone())
            }
            RightState::FileView { .. } => return,
        };

        if entry.name == ".." {
            if let Some(parent) = path.parent()
                && let Ok(entries) = read_entries(parent)
            {
                self.right_state = RightState::Browser {
                    entries,
                    cursor: 0,
                    path: parent.to_path_buf(),
                };
            }
            return;
        }

        let full_path = path.join(&entry.name);

        let is_dir = if entry.symlink_target.is_some() {
            fs::metadata(&full_path)
                .map(|m| m.is_dir())
                .unwrap_or(false)
        } else {
            entry.is_dir
        };

        if is_dir {
            if let Ok(entries) = read_entries(&full_path) {
                self.right_state = RightState::Browser {
                    entries,
                    cursor: 0,
                    path: full_path,
                };
            }
        } else {
            if let Ok(viewer) = FileViewer::new(&full_path) {
                self.right_state = RightState::FileView {
                    viewer,
                    path: full_path,
                };
            }
        }
    }

    fn on_tab(&mut self) {
        self.focus = self.focus.toggle();
    }
}
