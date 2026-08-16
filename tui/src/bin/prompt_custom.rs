use strum::EnumIter;

use tui::prompt::{PromptError, PromptOption, SelectPrompt};

#[derive(Clone, Debug, PartialEq, Eq, EnumIter)]
enum Choice {
    Bike,
    Tram,
    Carpool,
}

impl PromptOption for Choice {
    fn label(&self) -> Option<&str> {
        match self {
            Choice::Bike => Some("two wheeler"),
            Choice::Tram => Some("streetcar"),
            Choice::Carpool => Some("shared ride"),
        }
    }

    fn hotkey(&self) -> Option<char> {
        match self {
            Choice::Bike => Some('w'),
            Choice::Tram => Some('s'),
            Choice::Carpool => Some('z'),
        }
    }
}

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