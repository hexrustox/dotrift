use std::path::PathBuf;

use clap::{Parser, Subcommand};
use miette::{Result, WrapErr, miette};

use crate::ensure_absolute;

#[derive(Debug, Parser)]
#[command(name = "dotrift")]
pub struct Cli {
    #[arg(short, long)]
    pub source: Option<PathBuf>,
    #[arg(short, long)]
    pub target: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Status,
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProfileCommand {
    List,
    Activate { name: String },
    Deactivate { name: String },
    Show,
}

impl Cli {
    pub fn resolve(self) -> Result<(Option<PathBuf>, Command)> {
        let Cli {
            command, source, ..
        } = self;
        let source = if matches!(
            &command,
            Command::Status
                | Command::Profile {
                    command: ProfileCommand::Deactivate { .. }
                }
        ) {
            None
        } else {
            Some(match source {
                Some(path) => ensure_absolute(&path)?,
                None => ensure_absolute(&default_source()?)?,
            })
        };
        Ok((source, command))
    }
}

fn default_source() -> Result<PathBuf> {
    let source = dirs::data_dir()
        .map(|data_home| data_home.join("dotfiles"))
        .ok_or_else(|| miette!("both XDG_DATA_HOME and HOME are unset"))
        .wrap_err("cannot resolve source directory")?;
    Ok(source)
}
