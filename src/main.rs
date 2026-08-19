use clap::Parser;
use miette::{Error, miette};

use dotrift::{
    ExitStatus,
    cli::{Cli, Command},
};

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
            let Some(source) = source else {
                return Err(miette!("apply requires a source directory"));
            };
            status = dotrift::commands::apply::run_with_options(
                &source,
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
