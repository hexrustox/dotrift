mod diff;
mod explorer;
mod file_viewer;
mod footer;
mod view;

use std::{
    io::{self, BufRead, IsTerminal, Seek, SeekFrom, stdout},
    path::Path,
    sync::OnceLock,
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::Rect,
    widgets::{Block, Clear, Padding, Paragraph},
};

use diff::Diff;
use explorer::Explorer;
use view::View;

const HELP_TOP: &str = r#"j / ↓         Scroll down
k / ↑         Scroll up
Ctrl+D        Half page down
Ctrl+U        Half page up
PgDn / Ctrl+F Page down
PgUp / Ctrl+B Page up
g / Home      Jump to top
G / End       Jump to bottom"#;

const HELP_EXPLORER_EXTRAS: &str = r#"
Tab           Switch focus
Enter         Open entry / file
Esc           Go back"#;

const HELP_BOTTOM: &str = r#"
h             Toggle help
q / Ctrl+C    Quit

Press any key to close"#;

fn help_text(explorer: bool) -> String {
    let mut s = String::from(HELP_TOP);
    if explorer {
        s.push_str(HELP_EXPLORER_EXTRAS);
    }
    s.push_str(HELP_BOTTOM);
    s
}

#[derive(Clone)]
pub enum PagerArgs<'a> {
    View(&'a Path),
    Diff { source: &'a Path, target: &'a Path },
    Explorer { source: &'a Path, target: &'a Path },
}

pub fn run(arg: PagerArgs) -> io::Result<()> {
    if !io::stdin().is_terminal() {
        return Ok(());
    }

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = match arg {
        PagerArgs::View(path) => {
            let mut pane = View::new(path)?;
            run_app(&mut terminal, &mut pane, &help_text(false))
        }
        PagerArgs::Diff { source, target } => {
            let mut pane = Diff::new(target, source)?;
            run_app(&mut terminal, &mut pane, &help_text(false))
        }
        PagerArgs::Explorer { source, target } => {
            let mut pane = Explorer::new(source, target)?;
            run_app(&mut terminal, &mut pane, &help_text(true))
        }
    };

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    result
}

fn run_app<T: PagerMode>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    pager: &mut T,
    help_text: &str,
) -> io::Result<()> {
    let mut help_visible = false;
    loop {
        terminal.draw(|f| {
            pager.render(f, f.area());
            if help_visible {
                render_help_popup(f, f.area(), help_text);
            }
        })?;

        let event = event::read()?;
        if help_visible {
            if let Event::Key(key) = event
                && key.kind == KeyEventKind::Press
            {
                help_visible = false;
            }
            continue;
        }
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let viewport_h = pager.viewport_height().max(1);
                match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
                    KeyCode::Char('h') => help_visible = true,
                    KeyCode::Esc => pager.on_esc(),
                    KeyCode::Tab => pager.on_tab(),
                    KeyCode::Enter => pager.on_enter(),
                    KeyCode::Char('j') | KeyCode::Down => pager.scroll_down(1, viewport_h),
                    KeyCode::Char('k') | KeyCode::Up => pager.scroll_up(1),
                    KeyCode::PageDown | KeyCode::Char('f')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        pager.scroll_down(viewport_h, viewport_h);
                    }
                    KeyCode::PageUp | KeyCode::Char('b')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        pager.scroll_up(viewport_h);
                    }
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        pager.scroll_down(viewport_h / 2, viewport_h);
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        pager.scroll_up(viewport_h / 2);
                    }
                    KeyCode::Char('g') | KeyCode::Home => pager.scroll_to_top(),
                    KeyCode::Char('G') | KeyCode::End => pager.scroll_to_bottom(viewport_h),
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

trait PagerMode: Sized {
    fn render(&mut self, frame: &mut Frame, area: Rect);

    fn scroll_up(&mut self, _n: usize) {}
    fn scroll_down(&mut self, _n: usize, _viewport_h: usize) {}
    fn scroll_to_top(&mut self) {}
    fn scroll_to_bottom(&mut self, _viewport_h: usize) {}
    fn on_esc(&mut self) {}
    fn on_tab(&mut self) {}
    fn on_enter(&mut self) {}
    fn viewport_height(&self) -> usize {
        0
    }
}

struct Scroll(usize);

impl Scroll {
    fn new() -> Self {
        Self(0)
    }

    fn get(&self) -> usize {
        self.0
    }

    fn up(&mut self, n: usize) {
        self.0 = self.0.saturating_sub(n);
    }

    fn down(&mut self, n: usize, total: usize, vp: usize) {
        let max = total.saturating_sub(vp);
        self.0 = (self.0 + n).min(max);
    }

    fn top(&mut self) {
        self.0 = 0;
    }

    fn bottom(&mut self, total: usize, vp: usize) {
        self.0 = total.saturating_sub(vp);
    }

    fn clamp(&mut self, total: usize, vp: usize) {
        let max = total.saturating_sub(vp);
        self.0 = self.0.min(max);
    }
}

pub fn scroll_status(scroll_pos: usize, total: usize, viewport_h: usize) -> String {
    let max_pos = total.saturating_sub(viewport_h) + 1;
    format!("({}/{})", scroll_pos + 1, max_pos)
}

fn render_help_popup(frame: &mut Frame, screen: Rect, text: &str) {
    let lines = text.lines().count() as u16;
    let popup_w = (screen.width * 3 / 4).max(40);
    let popup_h = lines + 4;
    let area = center(screen, popup_w, popup_h);

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(text).block(
            Block::bordered()
                .title_top(" Help ")
                .padding(Padding::proportional(1)),
        ),
        area,
    );
}

fn center(screen: Rect, w: u16, h: u16) -> Rect {
    let x = screen.x + (screen.width.saturating_sub(w)) / 2;
    let y = screen.y + (screen.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

fn build_offsets(reader: &mut (impl BufRead + Seek + ?Sized)) -> io::Result<Vec<u64>> {
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

    reader.seek(SeekFrom::Start(0))?;
    Ok(offsets)
}

fn offsets_from_bytes(data: &[u8]) -> Vec<u64> {
    let mut offsets = vec![0u64];
    for (i, &b) in data.iter().enumerate() {
        if b == b'\n' {
            offsets.push(i as u64 + 1);
        }
    }
    let last = *offsets.last().unwrap() as usize;
    if last != data.len() {
        offsets.push(data.len() as u64);
    }
    offsets
}

fn is_unicode() -> bool {
    static UNICODE: OnceLock<bool> = OnceLock::new();
    *UNICODE.get_or_init(|| {
        ["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .filter_map(|v| std::env::var(v).ok())
            .any(|s| s.contains("UTF-8") || s.contains("utf8"))
    })
}

fn splitter_char() -> &'static str {
    if is_unicode() { "│" } else { "|" }
}

fn arrow_char() -> &'static str {
    if is_unicode() { "→" } else { "->" }
}

fn cursor_char() -> &'static str {
    if is_unicode() { "▶ " } else { "> " }
}
