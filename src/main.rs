mod cli;
mod config;

use clap::Parser;
use cli::{Cli, Commands};

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Apply(_flags) => {}
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
}
