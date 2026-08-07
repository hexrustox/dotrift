use std::fmt::Debug;

use strum::EnumIter;
use tui::prompt::{PromptError, PromptOption, SelectPrompt};

#[derive(Clone, Debug, PartialEq, Eq, EnumIter)]
enum Fruits {
    Apple,
    Banana,
    Orange,
}

impl PromptOption for Fruits {}

fn main() {
    match SelectPrompt::new()
        .question("what to eat")
        .default(Fruits::Apple)
        .interact()
    {
        Ok(action) => println!("picked: {action:?}"),
        Err(PromptError::Cancelled) => {
            eprintln!("cancelled");
            std::process::exit(130);
        }
        Err(error) => {
            eprintln!("error: {error:?}");
            std::process::exit(1);
        }
    }
}
