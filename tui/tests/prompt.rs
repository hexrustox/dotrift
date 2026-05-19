use std::fmt::Display;

use strum::EnumIter;
use tui::prompt::{HotKey, SelectPrompt};

#[derive(Default, PartialEq, EnumIter)]
enum Fruits {
    #[default]
    Apple,
    Banana,
    Carrot,
}

impl Display for Fruits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fruits::Apple => write!(f, "apple"),
            Fruits::Banana => write!(f, "banana"),
            Fruits::Carrot => write!(f, "carrot"),
        }
    }
}

impl HotKey for Fruits {
    fn hot_key(&self) -> char {
        match self {
            Fruits::Apple => 'a',
            Fruits::Banana => 'b',
            Self::Carrot => 'c',
        }
    }
}

#[ignore]
#[test]
fn main() {
    let select = SelectPrompt::<Fruits>::new()
        .prompt("What fruit? ")
        .interact()
        .unwrap();

    println!("{select}");
}
