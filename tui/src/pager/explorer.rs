use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::Line,
    widgets::{List, ListItem, ListState, Paragraph},
};

use crate::{ls_colors::LsColors, pager::compact_path};

use super::{
    PagerMode, arrow_char, cursor_char, file_viewer::FileViewer, footer, scroll_status,
    splitter_char,
};

struct DirEntry {
    name: String,
    is_dir: bool,
    symlink_target: Option<String>,
    mode: Option<u32>,
    is_broken: bool,
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
    Error {
        message: String,
        prev_path: PathBuf,
    },
}

impl BrowseState {
    fn current_path(&self) -> &PathBuf {
        match self {
            BrowseState::Dir { path, .. } | BrowseState::File { path, .. } => path,
            BrowseState::Error { prev_path, .. } => prev_path,
        }
    }
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
    path: PathBuf,
    viewport_h: usize,
    ls_colors: LsColors,
}

impl Explorer {
    pub fn new(file: &Path, dir: &Path) -> io::Result<Self> {
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
            path: dir.to_path_buf(),
            viewport_h: 0,
            ls_colors: LsColors::new(),
        })
    }

    fn navigate_to(&mut self, target: PathBuf, fallback: PathBuf) {
        match read_entries(&target) {
            Ok(entries) => {
                self.browser = BrowseState::Dir {
                    entries,
                    cursor: 0,
                    path: target,
                };
            }
            Err(e) => {
                self.browser = BrowseState::Error {
                    message: e.to_string(),
                    prev_path: fallback,
                };
            }
        }
    }
}

fn read_entries(path: &Path) -> io::Result<Vec<DirEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = ft.is_dir();
        let is_symlink = ft.is_symlink();
        let meta = entry.metadata();
        let symlink_target = if is_symlink {
            fs::read_link(entry.path())
                .ok()
                .map(|p| p.display().to_string())
        } else {
            None
        };
        let is_broken = is_symlink && meta.is_err();
        let mode = if is_symlink {
            None
        } else {
            meta.ok().map(|m| m.permissions().mode())
        };
        entries.push(DirEntry {
            name,
            is_dir,
            symlink_target,
            mode,
            is_broken,
        });
    }
    entries.sort_by(|a, b| {
        a.is_dir
            .cmp(&b.is_dir)
            .reverse()
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(entries)
}

impl PagerMode for Explorer {
    fn render(&mut self, frame: &mut Frame, area: Rect) {
        let [content_area, footer_area] = {
            let c = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(area);
            [c[0], c[1]]
        };
        self.viewport_h = content_area.height as usize;

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Ratio(1, 2),
                Constraint::Length(1),
                Constraint::Ratio(1, 2),
            ])
            .split(content_area);
        let mut browser_area = columns[0];
        let splitter_area = columns[1];
        let preview_area = columns[2];

        frame.render_widget(
            Paragraph::new(format!("{}\n", splitter_char()).repeat(splitter_area.height as usize)),
            splitter_area,
        );

        if let BrowseState::Dir { path, .. } = &self.browser {
            let cwd_layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Min(0)])
                .split(browser_area);
            let cwd_area = cwd_layout[0];
            browser_area = cwd_layout[1];

            frame.render_widget(
                Line::from(compact_path(path, cwd_area.width as usize)),
                cwd_area,
            );
        }

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
                            let is_symlink = e.symlink_target.is_some();
                            let style = self.ls_colors.style_for(
                                &e.name,
                                e.mode,
                                e.is_dir,
                                is_symlink,
                                e.is_broken,
                            );
                            ListItem::new(display).style(style)
                        })
                        .collect();

                    let list = List::new(items).highlight_symbol(if self.focus == Focus::Browser {
                        cursor_char()
                    } else {
                        "  "
                    });

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
            BrowseState::Error { message, .. } => {
                frame.render_widget(Paragraph::new(message.as_str()), browser_area);
            }
        }

        self.preview.render(frame, preview_area);

        let status_text = match self.focus {
            Focus::Browser => match &self.browser {
                BrowseState::Dir {
                    entries, cursor, ..
                } => {
                    format!("Browser ({}/{})", cursor + 1, entries.len())
                }
                BrowseState::File { viewer, .. } => {
                    let total = viewer.lines_count();
                    let vp_h = browser_area.height as usize;
                    format!(
                        "Browser {}",
                        scroll_status(viewer.scroll_pos(), total, vp_h)
                    )
                }
                BrowseState::Error { .. } => "Browser".to_string(),
            },
            Focus::Preview => {
                let total = self.preview.lines_count();
                let vp_h = preview_area.height as usize;
                format!(
                    "Preview {}",
                    scroll_status(self.preview.scroll_pos(), total, vp_h)
                )
            }
        };
        footer::render(frame, footer_area, &status_text, &self.path);
    }

    fn viewport_height(&self) -> usize {
        self.viewport_h
    }

    fn scroll_up(&mut self, n: usize) {
        match self.focus {
            Focus::Browser => match &mut self.browser {
                BrowseState::Dir { cursor, .. } => {
                    *cursor = cursor.saturating_sub(n);
                }
                BrowseState::File { viewer, .. } => viewer.scroll_up(n),
                BrowseState::Error { .. } => {}
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
                BrowseState::Error { .. } => {}
            },
            Focus::Preview => self.preview.scroll_down(n, viewport_h),
        }
    }

    fn scroll_to_top(&mut self) {
        match self.focus {
            Focus::Browser => match &mut self.browser {
                BrowseState::Dir { cursor, .. } => *cursor = 0,
                BrowseState::File { viewer, .. } => viewer.scroll_to_top(),
                BrowseState::Error { .. } => {}
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
                BrowseState::Error { .. } => {}
            },
            Focus::Preview => self.preview.scroll_to_bottom(viewport_h),
        }
    }

    fn on_esc(&mut self) {
        if self.focus != Focus::Browser {
            return;
        }
        let path = self.browser.current_path().clone();
        if let Some(parent) = path.parent() {
            self.navigate_to(parent.to_path_buf(), path);
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
            BrowseState::Error { .. } => return,
        };

        if entry.name == ".." {
            if let Some(parent) = path.parent() {
                self.navigate_to(parent.to_path_buf(), path);
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
            self.navigate_to(full_path, path);
        } else {
            match FileViewer::new(&full_path) {
                Ok(viewer) => {
                    self.browser = BrowseState::File {
                        viewer,
                        path: full_path,
                    };
                }
                Err(e) => {
                    self.browser = BrowseState::Error {
                        message: e.to_string(),
                        prev_path: path,
                    };
                }
            }
        }
    }

    fn on_tab(&mut self) {
        self.focus = self.focus.toggle();
    }
}
