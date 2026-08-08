use clap::Parser;

use dotrift::cli::{Cli, Command, state_root};
use dotrift::state::StateDatabase;

fn main() {
    if let Err(error) = run() {
        eprintln!("dotrift: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Status => {
            let Some(state_root) = state_root().ok() else {
                return Ok(());
            };
            let database = StateDatabase::open_read_only(state_root)?;
            for line in dotrift::status_lines(&database)? {
                println!("{line}");
            }
        }
    }
    Ok(())
}
