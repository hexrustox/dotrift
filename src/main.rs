use clap::Parser;
use miette::Error;

use dotrift::ExitStatus;
use dotrift::cli::{Cli, Command};

fn main() -> Result<(), Error> {
    let cli = Cli::parse();
    let (source, target, command) = cli.resolve()?;
    let mut status = ExitStatus::Success;
    match command {
        Command::Status => dotrift::commands::status::run()?,
        Command::Apply => {
            status = dotrift::commands::apply::run(
                &source.expect("apply source is unavailable"),
                target,
            )?
        }
        Command::Profile { command } => {
            dotrift::commands::profile::run(source.as_deref(), command)?
        }
    }
    std::process::exit(status as i32);
}
