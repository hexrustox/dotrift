use clap::Parser;
use miette::Error;

use dotrift::cli::{Cli, Command};

fn main() -> Result<(), Error> {
    let cli = Cli::parse();
    let (source, target, command) = cli.resolve()?;
    match command {
        Command::Status => dotrift::commands::status::run()?,
        Command::Apply => dotrift::commands::apply::run(
            source
                .as_deref()
                .ok_or_else(|| miette::miette!("apply source is unavailable"))?,
            target,
        )?,
        Command::Profile { command } => {
            dotrift::commands::profile::run(source.as_deref(), command)?
        }
    }
    Ok(())
}
