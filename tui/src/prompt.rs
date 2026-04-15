use std::{
    fmt::Display,
    io::{self, Write},
};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    queue,
    style::{Color, Print, Stylize},
    terminal,
};
use strum::IntoEnumIterator;

pub trait HotKey {
    fn hot_key(&self) -> char;
}

pub struct SelectPrompt<I> {
    prompt: String,
    default: Option<I>,
    separator: String,
    anchor: String,
}

impl<I> SelectPrompt<I>
where
    I: Default + HotKey + Display + IntoEnumIterator + PartialEq,
{
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            prompt: String::new(),
            default: None,
            separator: String::from("/"),
            anchor: String::from("> "),
        }
    }

    pub fn prompt(mut self, s: impl Into<String>) -> Self {
        self.prompt = s.into();
        self
    }

    pub fn separator(mut self, s: impl Into<String>) -> Self {
        self.separator = s.into();
        self
    }

    pub fn anchor(mut self, s: impl Into<String>) -> Self {
        self.anchor = s.into();
        self
    }

    pub fn interact(self) -> io::Result<I> {
        let mut stdout = io::stdout();
        queue!(stdout, cursor::Hide, cursor::SavePosition)?;
        stdout.flush()?;
        crossterm::terminal::enable_raw_mode()?;

        let mut select = self.default.unwrap_or_default();
        loop {
            queue!(
                stdout,
                Print(format!(
                    "{}{}",
                    self.prompt,
                    I::iter()
                        .map(|item| if item == select {
                            format!("{}{}", self.anchor, item)
                                .with(Color::Blue)
                                .bold()
                                .to_string()
                        } else {
                            format!("[{}]{}", item.hot_key(), item)
                        })
                        .collect::<Vec<_>>()
                        .join(&self.separator)
                ))
            )?;
            stdout.flush()?;

            loop {
                if let Event::Key(key_event) = event::read()? {
                    match key_event.code {
                        KeyCode::Char(c) => {
                            if let Some(s) = I::iter().find(|item| item.hot_key() == c) {
                                select = s;

                                queue!(
                                    stdout,
                                    cursor::RestorePosition,
                                    terminal::Clear(terminal::ClearType::CurrentLine),
                                )?;
                                stdout.flush()?;
                                break;
                            }
                        }
                        KeyCode::Enter => {
                            crossterm::terminal::disable_raw_mode()?;
                            queue!(
                                stdout,
                                cursor::RestorePosition,
                                terminal::Clear(terminal::ClearType::FromCursorDown),
                                cursor::Show,
                            )?;
                            stdout.flush()?;

                            return Ok(select);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}
