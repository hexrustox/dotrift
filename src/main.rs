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
        Command::Apply {
            clean_up,
            prune_empty_dirs,
            dry_run,
            quiet,
            verbose,
        } => {
            status = dotrift::commands::apply::run_with_options(
                &source.expect("apply source is unavailable"),
                target,
                dotrift::commands::apply::ApplyOptions {
                    clean_up,
                    prune_empty_dirs,
                    dry_run,
                    quiet,
                    verbose,
                },
            )?
        }
        Command::Profile { command } => {
            dotrift::commands::profile::run(source.as_deref(), command)?
        }
    }
    std::process::exit(status as i32);
}
