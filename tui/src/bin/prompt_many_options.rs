use strum::EnumIter;

use tui::prompt::{PromptError, PromptOption, SelectPrompt};

#[derive(Clone, Debug, PartialEq, Eq, EnumIter)]
enum Choice {
    Bike,
    Carpool,
    EVScooter,
    Metro,
    Tram,
}

impl PromptOption for Choice {}

fn main() {
    match SelectPrompt::<Choice>::new()
        .question("Pick an option")
        .interact()
    {
        Ok(choice) => println!("confirmed: {choice:?}"),
        Err(PromptError::Cancelled) => println!("cancelled"),
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    }
}
