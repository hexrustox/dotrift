use clap::Parser;
use miette::{Error, miette};

use dotrift::{
    ExitStatus,
    cli::{Cli, Command},
    environment::Environment,
};

fn main() -> Result<(), Error> {
    let color = tui::color_support();

    let env = Environment::resolve();
    let cli = Cli::parse();
    let (source, target, command) = cli.resolve(&env)?;
    let mut status = ExitStatus::Success;
    match command {
        Command::Status => dotrift::commands::status::run(&env, color)?,
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
                &env,
                color,
            )?
        }
        Command::Profile { command } => {
            dotrift::commands::profile::run(source.as_deref(), command, &env, color)?
        }
    }
    std::process::exit(status as i32);
}
