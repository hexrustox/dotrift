use std::{fmt::Display, path::Path};

use color_eyre::Result;
use strum::EnumIter;
use tui::prompt::HotKey;

#[derive(Default, Clone, Copy, PartialEq, EnumIter)]
pub enum CollisionOptions {
    #[default]
    Skip,
    Overwrite,
    Diff,
    Quit,
}

impl Display for CollisionOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use CollisionOptions::*;
        match self {
            Skip => write!(f, "skip"),
            Overwrite => write!(f, "overwrite"),
            Diff => write!(f, "diff"),
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
            Diff => 'd',
            Quit => 'q',
        }
    }
}

#[cfg(test)]
pub fn prompt_collision(_: &Path, _: bool) -> Result<CollisionOptions> {
    Ok(tests::PROMPT_SELECTION.with_borrow(|n| *n))
}

#[cfg(not(test))]
pub fn prompt_collision(path: &Path, is_dir: bool) -> Result<CollisionOptions> {
    use color_eyre::eyre::Context;
    use tui::prompt::SelectPrompt;

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
