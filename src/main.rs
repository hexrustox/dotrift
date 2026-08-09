use clap::Parser;
use miette::Error;

use dotrift::cli::{Cli, Command};

fn main() -> Result<(), Error> {
    let cli = Cli::parse();
    match cli.command {
        Command::Status => dotrift::commands::status::run()?,
    }
    Ok(())
}
