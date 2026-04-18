use std::{
    fmt::Display,
    io::{self, IsTerminal},
    path::Path,
};

use color_eyre::eyre::Context;
use strum::EnumIter;
use tui::prompt::{HotKey, SelectPrompt};

#[derive(Default, Clone, Copy, PartialEq, EnumIter)]
pub enum CollisionOptions {
    #[default]
    Skip,
    Overwrite,
    Quit,
}

impl Display for CollisionOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use CollisionOptions::*;
        match self {
            Skip => write!(f, "skip"),
            Overwrite => write!(f, "overwrite"),
            Quit => write!(f, "quit"),
        }
    }
}

impl HotKey for CollisionOptions {
    fn hot_key(&self) -> char {
        use CollisionOptions::*;
        match self {
            Skip => 's',
            Overwrite => 'o',
            Quit => 'q',
        }
    }
}

#[allow(unused_variables)]
pub fn prompt_collision(path: &Path, is_dir: bool) -> color_eyre::Result<CollisionOptions> {
    #[cfg(test)]
    {
        use crate::command::prompt::tests::PROMPT_SELECTION;

        return Ok(PROMPT_SELECTION.with_borrow(|n| *n));
    }

    #[allow(unreachable_code)]
    let stdin = io::stdin();
    if !stdin.is_terminal() {
        return Ok(CollisionOptions::default());
    }

    let type_str = if is_dir { "directory" } else { "file" };
    SelectPrompt::new()
        .prompt(format!(
            "`{}` is an existing {}, ",
            path.display(),
            type_str
        ))
        .interact()
        .wrap_err("Failed to get user input")
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        pub static PROMPT_SELECTION: RefCell<CollisionOptions> = RefCell::new(CollisionOptions::default()) ;
    }
}
