pub(crate) mod explorer;
pub(crate) mod file_viewer;
pub(crate) mod header;
pub(crate) mod side_by_side;
pub(crate) mod single_pane;

use std::{
    io::{self, BufRead, IsTerminal, Seek, SeekFrom, stdout},
    path::Path,
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Frame, Terminal, backend::CrosstermBackend, layout::Rect};

use explorer::Explorer;
use side_by_side::SideBySide;
use single_pane::SinglePane;

pub fn run(path1: &Path, path2: Option<&Path>) -> io::Result<()> {
    if !io::stdin().is_terminal() {
        return Ok(());
    }

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = match path2 {
        Some(path2) if path1.is_file() && path2.is_dir() => {
            let mut pane = Explorer::new(path1, path2)?;
            run_app(&mut terminal, &mut pane)
        }
        Some(path2) => {
            let mut pane = SideBySide::new(path1, path2)?;
            run_app(&mut terminal, &mut pane)
        }
        None => {
            let mut pane = SinglePane::new(path1)?;
            run_app(&mut terminal, &mut pane)
        }
    };

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    result
}

fn run_app<T: PagerMode>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    pager: &mut T,
) -> io::Result<()> {
    loop {
        terminal.draw(|f| pager.render(f, f.area()))?;

        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let viewport_h = (terminal.size()?.height.saturating_sub(1) as usize).max(1);
                match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
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

fn build_offsets(reader: &mut (impl BufRead + Seek + ?Sized)) -> io::Result<(Vec<u64>, Vec<u8>)> {
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
    Ok((offsets, buf))
}

fn offsets_from_bytes(data: &[u8]) -> Vec<u64> {
    let mut offsets = vec![0u64];
    for (i, &b) in data.iter().enumerate() {
        if b == b'\n' {
            offsets.push(i as u64 + 1);
        }
    }
    offsets
}
