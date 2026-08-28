use std::fmt::Display;

use crossterm::style::{Color, Stylize};

pub mod prompt;

pub fn color_support() -> bool {
    supports_color::on(supports_color::Stream::Stdout).is_some()
}

pub fn apply_color<T>(content: T, color: Color, enabled: bool) -> String
where
    T: Display + Stylize,
    T::Styled: Display,
{
    if enabled {
        content.with(color).to_string()
    } else {
        content.to_string()
    }
}
