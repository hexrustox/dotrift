use std::{fs, io, path::PathBuf};

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    widgets::{List, ListItem, ListState, Paragraph},
};

use super::{PagerMode, arrow_char, cursor_char, file_viewer::FileViewer, header, splitter_char};

struct DirEntry {
    name: String,
    is_dir: bool,
    symlink_target: Option<String>,
}

enum BrowseState {
    Dir {
        entries: Vec<DirEntry>,
        cursor: usize,
        path: PathBuf,
    },
    File {
        viewer: FileViewer,
        path: PathBuf,
    },
}

#[derive(Clone, Copy, PartialEq)]
enum Focus {
    Browser,
    Preview,
}

impl Focus {
    fn toggle(self) -> Self {
        match self {
            Focus::Browser => Focus::Preview,
            Focus::Preview => Focus::Browser,
        }
    }
}

pub struct Explorer {
    preview: FileViewer,
    browser: BrowseState,
    focus: Focus,
    header: String,
}

impl Explorer {
    pub fn new(file: &std::path::Path, dir: &std::path::Path) -> io::Result<Self> {
        let preview = FileViewer::new(file)?;
        let entries = read_entries(dir)?;
        let browser = BrowseState::Dir {
            entries,
            cursor: 0,
            path: dir.to_path_buf(),
        };
        Ok(Self {
            preview,
            browser,
            focus: Focus::Browser,
            header: format!("Directory {} blocks file creation", dir.display()),
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

        header::render(frame, header_area, &self.header);

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Ratio(1, 2),
                Constraint::Length(1),
                Constraint::Ratio(1, 2),
            ])
            .split(content_area);
        let browser_area = columns[0];
        let splitter_area = columns[1];
        let preview_area = columns[2];

        frame.render_widget(
            Paragraph::new(format!("{}\n", splitter_char()).repeat(splitter_area.height as usize)),
            splitter_area,
        );

        match &mut self.browser {
            BrowseState::Dir {
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
                                Some(t) => format!("{} {} {}", e.name, arrow_char(), t),
                                None if e.is_dir => format!("{}/", e.name),
                                None => e.name.clone(),
                            };
                            ListItem::new(display)
                        })
                        .collect();

                    let list = List::new(items)
                        .highlight_symbol(if self.focus == Focus::Browser {
                            cursor_char()
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
            BrowseState::File { viewer, .. } => {
                viewer.render(frame, browser_area);
            }
        }

        self.preview.render(frame, preview_area);
    }

    fn scroll_up(&mut self, n: usize) {
        match self.focus {
            Focus::Browser => match &mut self.browser {
                BrowseState::Dir { cursor, .. } => {
                    *cursor = cursor.saturating_sub(n);
                }
                BrowseState::File { viewer, .. } => viewer.scroll_up(n),
            },
            Focus::Preview => self.preview.scroll_up(n),
        }
    }

    fn scroll_down(&mut self, n: usize, viewport_h: usize) {
        match self.focus {
            Focus::Browser => match &mut self.browser {
                BrowseState::Dir {
                    entries, cursor, ..
                } => {
                    let max = entries.len().saturating_sub(1);
                    *cursor = (*cursor + n).min(max);
                }
                BrowseState::File { viewer, .. } => viewer.scroll_down(n, viewport_h),
            },
            Focus::Preview => self.preview.scroll_down(n, viewport_h),
        }
    }

    fn scroll_to_top(&mut self) {
        match self.focus {
            Focus::Browser => match &mut self.browser {
                BrowseState::Dir { cursor, .. } => *cursor = 0,
                BrowseState::File { viewer, .. } => viewer.scroll_to_top(),
            },
            Focus::Preview => self.preview.scroll_to_top(),
        }
    }

    fn scroll_to_bottom(&mut self, viewport_h: usize) {
        match self.focus {
            Focus::Browser => match &mut self.browser {
                BrowseState::Dir {
                    entries, cursor, ..
                } => {
                    *cursor = entries.len().saturating_sub(1);
                }
                BrowseState::File { viewer, .. } => viewer.scroll_to_bottom(viewport_h),
            },
            Focus::Preview => self.preview.scroll_to_bottom(viewport_h),
        }
    }

    fn on_esc(&mut self) {
        if self.focus != Focus::Browser {
            return;
        }
        match &self.browser {
            BrowseState::Dir { path, .. } => {
                if let Some(parent) = path.parent()
                    && let Ok(entries) = read_entries(parent)
                {
                    self.browser = BrowseState::Dir {
                        entries,
                        cursor: 0,
                        path: parent.to_path_buf(),
                    };
                }
            }
            BrowseState::File { path, .. } => {
                let parent = path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| path.clone());
                if let Ok(entries) = read_entries(&parent) {
                    self.browser = BrowseState::Dir {
                        entries,
                        cursor: 0,
                        path: parent,
                    };
                }
            }
        }
    }

    fn on_enter(&mut self) {
        if self.focus != Focus::Browser {
            return;
        }
        let (entry, path) = match &self.browser {
            BrowseState::Dir {
                entries,
                cursor,
                path,
            } => {
                let Some(entry) = entries.get(*cursor) else {
                    return;
                };
                (entry, path.clone())
            }
            BrowseState::File { .. } => return,
        };

        if entry.name == ".." {
            if let Some(parent) = path.parent()
                && let Ok(entries) = read_entries(parent)
            {
                self.browser = BrowseState::Dir {
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
                self.browser = BrowseState::Dir {
                    entries,
                    cursor: 0,
                    path: full_path,
                };
            }
        } else {
            if let Ok(viewer) = FileViewer::new(&full_path) {
                self.browser = BrowseState::File {
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
