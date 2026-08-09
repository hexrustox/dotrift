use clap::Parser;
use miette::Error;

use dotrift::cli::{Cli, Command, ProfileCommand};

fn main() -> Result<(), Error> {
    let cli = Cli::parse();
    if matches!(&cli.command, Command::Status) {
        dotrift::commands::status::run()?;
        return Ok(());
    }
    if matches!(
        &cli.command,
        Command::Profile {
            command: ProfileCommand::Deactivate { .. }
        }
    ) {
        if let Command::Profile { command } = cli.command {
            dotrift::commands::profile::run(None, command)?;
        }
        return Ok(());
    }
    let (source, command) = cli.resolve_source()?;
    match command {
        Command::Profile { command } => dotrift::commands::profile::run(Some(&source), command)?,
        Command::Status => unreachable!(),
    }
    Ok(())
}
