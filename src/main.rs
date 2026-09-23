use clap::Parser;

use dotrift::{cli::Command, commands::require_source};

fn main() -> miette::Result<()> {
    let color = tui::color_support();

    let env = dotrift::platform::Environment::resolve();
    let cli = dotrift::cli::Cli::parse();
    let (source, target, command) = cli.resolve(&env)?;
    let mut status = dotrift::ExitStatus::Success;
    match command {
        Command::Status => dotrift::commands::status::run(&env, color)?,
        Command::Init => {
            dotrift::commands::init::run(require_source("init", source.as_deref())?, &env, color)?
        }
        Command::Apply {
            clean_up,
            prune_empty_dirs,
            dry_run,
            quiet,
            verbose,
        } => {
            status = dotrift::commands::apply::run_with_options(
                require_source("apply", source.as_deref())?,
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
