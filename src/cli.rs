use std::path::PathBuf;

use clap::{Parser, Subcommand};
use miette::{Result, miette};

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
}

#[derive(Debug, Clone)]
pub struct ResolvedGlobalOptions {
    pub source: PathBuf,
    pub target: PathBuf,
}

impl Cli {
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
    dirs::data_dir()
        .map(|data_home| data_home.join("dotfiles"))
        .ok_or_else(|| {
            miette!("cannot resolve source directory: both XDG_DATA_HOME and HOME are unset")
        })
}

fn default_target() -> Result<PathBuf> {
    dirs::home_dir()
        .ok_or_else(|| miette!("cannot resolve target directory: HOME is unset or empty"))
}
