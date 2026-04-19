use std::{env::current_dir, path::PathBuf};

use clap::Parser;
use cli::{Cli, Commands};
use color_eyre::eyre::Context;
use dotrift::{
    cli,
    command::{add, apply, init, status, unapply},
    path::{db_path, source_path},
};
use normalize_path::NormalizePath;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let mut cli = Cli::parse();

    if let Some(path) = cli.source {
        cli.source = Some(full_path(path)?);
    }
    if let Some(path) = cli.target {
        cli.target = Some(full_path(path)?);
    }

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
            .wrap_err("Failed to add path")?,
        Commands::Diff { .. } => {}
        Commands::Status { file } => {
            status::run(file, &db_path()).wrap_err("Failed to get status")?;
        }
    }

    Ok(())
}

fn full_path(path: PathBuf) -> color_eyre::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        let cwd = current_dir().wrap_err("Failed to get current directory")?;
        Ok(cwd.join(path).normalize())
    }
}
