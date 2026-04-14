mod cli;
mod command;
mod config;
mod db;
mod path;

use clap::Parser;
use cli::{Cli, Commands};

use crate::path::{db_path, source_path};

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let cli = Cli::parse();

    let source_dir = cli.source.unwrap_or(source_path());

    match cli.command {
        Commands::Apply(flags) => {
            command::apply::run(source_dir, cli.target, &db_path(), flags)?;
        }
        Commands::Unapply(_flags) => {}
        Commands::Add {
            target_file: _,
            source_relative: _,
        } => {}
        Commands::Diff {
            target_file: _,
            extra_args: _,
        } => {}
        Commands::Status { target_file: _ } => {}
    }

    Ok(())
}
