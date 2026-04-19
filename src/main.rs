use std::{env::current_dir, path::PathBuf};

use clap::Parser;
use cli::{Cli, Commands};
use color_eyre::eyre::Context;
use dotrift::{
    cli::{self, StatusSubcommand},
    command::{add, apply, init, status, unapply},
    path::{db_path, source_path},
};
use normalize_path::NormalizePath;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let mut cli = Cli::parse();

    cli.source = full_path(cli.source)?;
    cli.target = full_path(cli.target)?;
    cli.config = full_path(cli.config)?;

    let source_dir = cli.source.unwrap_or(source_path());

    match cli.command {
        Commands::Init => {
            init::run(source_dir).wrap_err("Failed to initialize")?;
        }
        Commands::Apply(flags) => {
            apply::run(source_dir, cli.target, cli.config, &db_path(), flags)
                .wrap_err("Failed to apply dotfiles")?;
        }
        Commands::Unapply(flags) => {
            unapply::run(source_dir, cli.target, &db_path(), flags)
                .wrap_err("Failed to unapply dotfiles")?;
        }
        Commands::Add {
            flags,
            file,
            destination,
        } => add::run(source_dir, cli.config, file, destination, flags)
            .wrap_err("Failed to add file")?,
        Commands::Diff { .. } => {}
        Commands::Status { command } => match command {
            StatusSubcommand::List { file } => {
                status::list(full_path(file)?, &db_path()).wrap_err("Failed to list status")?;
            }
            StatusSubcommand::Clear { file } => {
                status::clear(full_path(file)?, &db_path()).wrap_err("Failed to clear status")?;
            }
        },
    }

    Ok(())
}

fn full_path(path: Option<PathBuf>) -> color_eyre::Result<Option<PathBuf>> {
    if let Some(path) = path {
        if path.is_absolute() {
            Ok(Some(path.normalize()))
        } else {
            let cwd = current_dir().wrap_err("Failed to get current directory")?;
            Ok(Some(cwd.join(path).normalize()))
        }
    } else {
        Ok(None)
    }
}
