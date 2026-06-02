use std::{fmt::Display, path::Path};

use strum::EnumIter;
use tui::{
    pager::PagerArgs,
    prompt::{HotKey, SelectPrompt},
};

use crate::{command::util::PathExt, output::print_warn};

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

#[allow(unused_variables, unreachable_code)]
pub fn prompt_collision(
    source: Option<&Path>,
    target: &Path,
    create_dir: bool,
    existing_dir: bool,
) -> CollisionOptions {
    #[cfg(test)]
    return tests::PROMPT_SELECTION.with_borrow(|n| *n);

    let type_str = |b| {
        if b { "directory" } else { "file" }
    };
    let msg = format!(
        "trying to create {} {} but another {} already exists",
        type_str(create_dir),
        target.display(),
        type_str(existing_dir)
    );
    let arg = match (source, target) {
        (None, p) => PagerArgs::View(p),
        (Some(s), t) if s.path_is_file() && t.path_is_file() => PagerArgs::Diff {
            source: s,
            target: t,
        },
        (Some(s), t) if s.path_is_file() && t.path_is_dir() => PagerArgs::Explorer {
            source: s,
            target: t,
        },
        _ => {
            #[cfg(test)]
            unreachable!();
            return CollisionOptions::default();
        }
    };

    loop {
        match SelectPrompt::new().prompt(&msg).interact() {
            Ok(CollisionOptions::Diff) => {
                if let Err(e) = tui::pager::run(arg.clone()) {
                    print_warn(format!("failed to open pager: {e}, skipping"));
                }
            }
            Ok(o) => {
                return o;
            }
            Err(e) => {
                print_warn(format!("failed to display prompt: {e}, skipping"));
                return CollisionOptions::Skip;
            }
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        pub static PROMPT_SELECTION: RefCell<CollisionOptions> = RefCell::new(CollisionOptions::default()) ;
    }
}
