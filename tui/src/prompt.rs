use std::{
    fmt::Display,
    io::{self, IsTerminal, Write},
};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    queue,
    style::{Attribute, Color, Print, Stylize},
    terminal,
};
use strum::IntoEnumIterator;

pub trait HotKey: Display {
    fn hot_key(&self) -> char;

    fn display(&self) -> String {
        let ch = self.hot_key().to_string();
        format!(
            "[{}]{}",
            ch,
            if let Some(s) = self.to_string().strip_prefix(&ch) {
                s.to_string()
            } else {
                self.to_string()
            }
        )
    }
}

pub struct SelectPrompt<I> {
    prompt: String,
    default: Option<I>,
    separator: String,
    anchor: String,
}

impl<I> SelectPrompt<I>
where
    I: Default + Clone + HotKey + Display + IntoEnumIterator + PartialEq,
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

    pub fn default(mut self, default: I) -> Self {
        self.default = Some(default);
        self
    }

    pub fn interact(self) -> io::Result<I> {
        let variants: Vec<I> = I::iter().collect();
        let len = variants.len();
        let mut select = self.default.unwrap_or_default();
        let mut index = 0;

        let stdin = io::stdin();
        if !stdin.is_terminal() {
            return Ok(select);
        }

        let mut stdout = io::stdout();
        queue!(stdout, cursor::Hide, cursor::SavePosition)?;
        stdout.flush()?;
        crossterm::terminal::enable_raw_mode()?;

        loop {
            queue!(
                stdout,
                Print(format!(
                    "{}({})",
                    self.prompt,
                    variants
                        .iter()
                        .map(|item| if *item == select {
                            format!("{}{}", self.anchor, item)
                                .with(Color::Blue)
                                .attribute(Attribute::Bold)
                                .to_string()
                        } else {
                            item.display().attribute(Attribute::Dim).to_string()
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
                            if let Some(pos) = variants.iter().position(|item| item.hot_key() == c)
                            {
                                select = variants[pos].clone();
                                index = pos;
                                break;
                            }
                        }
                        KeyCode::Left => {
                            index = (index + len - 1) % len;
                            select = variants[index].clone();
                            break;
                        }
                        KeyCode::Right => {
                            index = (index + 1) % len;
                            select = variants[index].clone();
                            break;
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

            queue!(
                stdout,
                cursor::RestorePosition,
                terminal::Clear(terminal::ClearType::CurrentLine),
            )?;
            stdout.flush()?;
        }
    }
}
