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

#[derive(Debug, Clone)]
pub struct ResolvedGlobalOptions {
    pub source: PathBuf,
    pub target: PathBuf,
}

impl Cli {
    pub fn resolve_source(self) -> Result<(PathBuf, Command)> {
        let source = match self.source {
            Some(path) => ensure_absolute(&path)?,
            None => ensure_absolute(&default_source()?)?,
        };
        Ok((source, self.command))
    }

    pub fn resolve_paths(self) -> Result<(ResolvedGlobalOptions, Command)> {
        let source = match self.source {
            Some(path) => ensure_absolute(&path)?,
            None => ensure_absolute(&default_source()?)?,
        };
        let target = match self.target {
            Some(path) => ensure_absolute(&path)?,
            None => ensure_absolute(&default_target()?)?,
        };

        Ok((ResolvedGlobalOptions { source, target }, self.command))
    }
}

fn default_source() -> Result<PathBuf> {
    let source = dirs::data_dir()
        .map(|data_home| data_home.join("dotfiles"))
        .ok_or_else(|| miette!("both XDG_DATA_HOME and HOME are unset"))
        .wrap_err("cannot resolve source directory")?;
    Ok(source)
}

fn default_target() -> Result<PathBuf> {
    let target = dirs::home_dir()
        .ok_or_else(|| miette!("HOME is unset or empty"))
        .wrap_err("cannot resolve target directory")?;
    Ok(target)
}
