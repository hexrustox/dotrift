use clap::Parser;
use miette::Error;

use dotrift::cli::{Cli, Command};

fn main() -> Result<(), Error> {
    let cli = Cli::parse();
    let (source, command) = cli.resolve()?;
    match command {
        Command::Status => dotrift::commands::status::run()?,
        Command::Profile { command } => {
            dotrift::commands::profile::run(source.as_deref(), command)?
        }
    }
    Ok(())
}
