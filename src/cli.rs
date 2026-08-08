use std::path::PathBuf;

use clap::{Parser, Subcommand};

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
    pub fn resolve_paths(self) -> Result<(ResolvedGlobalOptions, Command), String> {
        let source = match self.source {
            Some(path) => ensure_absolute(&path).map_err(|error| error.to_string())?,
            None => ensure_absolute(&default_source()?).map_err(|error| error.to_string())?,
        };
        let target = match self.target {
            Some(path) => ensure_absolute(&path).map_err(|error| error.to_string())?,
            None => ensure_absolute(&default_target()?).map_err(|error| error.to_string())?,
        };

        Ok((ResolvedGlobalOptions { source, target }, self.command))
    }
}

fn default_source() -> Result<PathBuf, String> {
    if let Some(data_home) = non_empty_env("XDG_DATA_HOME") {
        return Ok(data_home.join("dotfiles"));
    }
    if let Some(home) = non_empty_env("HOME") {
        return Ok(home.join(".local/share/dotfiles"));
    }
    Err("cannot resolve source directory: both XDG_DATA_HOME and HOME are unset".into())
}

fn default_target() -> Result<PathBuf, String> {
    non_empty_env("HOME")
        .ok_or_else(|| "cannot resolve target directory: HOME is unset or empty".into())
}

fn non_empty_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn state_root() -> Result<PathBuf, String> {
    if let Some(state_home) = non_empty_env("XDG_STATE_HOME") {
        return Ok(state_home.join("dotrift"));
    }
    if let Some(data_home) = non_empty_env("XDG_DATA_HOME") {
        return Ok(data_home.join("dotrift"));
    }
    Err("cannot resolve state location: XDG_STATE_HOME and XDG_DATA_HOME are unset".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_cli_paths_are_resolved_against_the_working_directory() {
        let cli = Cli::parse_from([
            "dotrift", "--source", "source", "--target", "target", "status",
        ]);
        let (options, _) = cli.resolve_paths().unwrap();

        let current = std::env::current_dir().unwrap();
        assert_eq!(options.source, current.join("source"));
        assert_eq!(options.target, current.join("target"));
    }
}
