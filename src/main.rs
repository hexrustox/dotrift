use clap::Parser;
use cli::{Cli, Commands};
use color_eyre::eyre::Context;
use dotrift::{
    cli::{self, StatusSubcommand},
    command::{add, apply, init, status, unapply},
    path::db_path,
};

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let cli = Cli::parse();

    match cli.command {
        Commands::Init => {
            init::run(cli.global).wrap_err("Failed to initialize source directory")?;
        }
        Commands::Apply(flags) => {
            apply::run(cli.global, &db_path(), flags).wrap_err("Failed to apply dotfiles")?;
        }
        Commands::Unapply(flags) => {
            unapply::run(cli.global, &db_path(), flags).wrap_err("Failed to unapply dotfiles")?;
        }
        Commands::Add {
            flags,
            path,
            destination,
        } => add::run(cli.global, path.clone(), destination, flags, &db_path())
            .wrap_err_with(|| format!("Failed to add `{}`", path.display()))?,
        Commands::Status { command } => match command {
            StatusSubcommand::List { file } => {
                status::list(file, &db_path()).wrap_err("Failed to list managed files")?;
            }
            StatusSubcommand::Clear { file } => {
                status::clear(file, &db_path()).wrap_err("Failed to clear managed files")?;
            }
        },
    }

    Ok(())
}
