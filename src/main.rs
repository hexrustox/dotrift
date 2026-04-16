use clap::Parser;
use cli::{Cli, Commands};
use color_eyre::eyre::Context;
use dotrift::{
    cli,
    command::apply,
    path::{db_path, source_path},
};

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let cli = Cli::parse();

    let source_dir = cli.source.unwrap_or(source_path());

    match cli.command {
        Commands::Apply(flags) => {
            apply::run(source_dir, cli.target, &db_path(), flags)
                .wrap_err("Failed to apply dotfiles")?;
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
