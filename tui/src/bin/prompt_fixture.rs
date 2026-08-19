use strum::EnumIter;

use tui::prompt::{PromptError, PromptOption, SelectPrompt};

#[derive(Clone, Debug, PartialEq, Eq, EnumIter)]
enum BasicChoice {
    Bike,
    Metro,
    Tram,
}

impl PromptOption for BasicChoice {}

#[derive(Clone, Debug, PartialEq, Eq, EnumIter)]
enum ManyChoice {
    Bike,
    Carpool,
    EVScooter,
    Metro,
    Tram,
}

impl PromptOption for ManyChoice {}

#[derive(Clone, Debug, PartialEq, Eq, EnumIter)]
enum CustomChoice {
    Bike,
    Tram,
    Carpool,
}

impl PromptOption for CustomChoice {
    fn label(&self) -> Option<&str> {
        match self {
            CustomChoice::Bike => Some("two wheeler"),
            CustomChoice::Tram => Some("streetcar"),
            CustomChoice::Carpool => Some("shared ride"),
        }
    }

    fn hotkey(&self) -> Option<char> {
        match self {
            CustomChoice::Bike => Some('w'),
            CustomChoice::Tram => Some('s'),
            CustomChoice::Carpool => Some('z'),
        }
    }
}

fn summarize<E: std::fmt::Debug>(result: Result<E, PromptError>) {
    match result {
        Ok(choice) => println!("confirmed: {choice:?}"),
        Err(PromptError::Cancelled) => println!("cancelled"),
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    }
}

fn main() {
    let fixture = std::env::args().nth(1).expect("fixture name required");
    match fixture.as_str() {
        "basic" => summarize(
            SelectPrompt::<BasicChoice>::new()
                .question("Pick an option")
                .interact(),
        ),
        "default" => summarize(
            SelectPrompt::<BasicChoice>::new()
                .question("Pick an option")
                .default(BasicChoice::Metro)
                .interact(),
        ),
        "many" => summarize(
            SelectPrompt::<ManyChoice>::new()
                .question("Pick an option")
                .interact(),
        ),
        "custom" => summarize(
            SelectPrompt::<CustomChoice>::new()
                .question("Pick an option")
                .interact(),
        ),
        "multiline" => summarize(
            SelectPrompt::<BasicChoice>::new()
                .question("Pick an option\n(you can change it)")
                .interact(),
        ),
        other => {
            eprintln!("error: unknown fixture {other}");
            std::process::exit(2);
        }
    }
}
