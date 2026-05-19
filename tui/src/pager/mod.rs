pub mod file_viewer;
pub mod header;
pub mod single_pane;

use std::{
    io::{self, IsTerminal, stdout},
    path::Path,
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Frame, Terminal, backend::CrosstermBackend, layout::Rect};

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

    let result = if path2.is_none() {
        let mut pane = SinglePane::new(path1)?;
        run_app(&mut terminal, &mut pane)
    } else {
        todo!()
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
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(());
                    }
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

pub trait PagerMode: Sized {
    fn render(&mut self, frame: &mut Frame, area: Rect);

    fn scroll_up(&mut self, _n: usize) {}
    fn scroll_down(&mut self, _n: usize, _viewport_h: usize) {}
    fn scroll_to_top(&mut self) {}
    fn scroll_to_bottom(&mut self, _viewport_h: usize) {}
}
