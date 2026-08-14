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
    Apply {
        #[arg(long)]
        clean_up: bool,
        #[arg(long)]
        prune_empty_dirs: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long, conflicts_with = "verbose")]
        quiet: bool,
        #[arg(long, conflicts_with = "quiet")]
        verbose: bool,
    },
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
    pub fn resolve(self) -> Result<(Option<PathBuf>, Option<PathBuf>, Command)> {
        let Cli {
            command,
            source,
            target,
        } = self;
        if let Command::Apply {
            prune_empty_dirs: true,
            clean_up: false,
            ..
        } = &command
        {
            return Err(miette::MietteDiagnostic::new(
                "`--prune-empty-dirs` cannot be used without `--clean-up`",
            )
            .with_help("pass `--clean-up` with `--prune-empty-dirs`")
            .into());
        }
        if let Command::Apply {
            dry_run: true,
            quiet: true,
            ..
        } = &command
        {
            return Err(miette!("`--dry-run` conflicts with `--quiet`"));
        }
        if let Command::Apply {
            dry_run: true,
            verbose: true,
            ..
        } = &command
        {
            return Err(miette!("`--dry-run` conflicts with `--verbose`"));
        }
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
        let target = target.map(|path| ensure_absolute(&path)).transpose()?;
        Ok((source, target, command))
    }
}

fn default_source() -> Result<PathBuf> {
    let source = dirs::data_dir()
        .map(|data_home| data_home.join("dotfiles"))
        .ok_or_else(|| miette!("both XDG_DATA_HOME and HOME are unset"))
        .wrap_err("cannot resolve source directory")?;
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_requires_cleanup() {
        let cli = Cli::try_parse_from(["dotrift", "apply", "--prune-empty-dirs"]).unwrap();
        assert!(cli.resolve().is_err());
    }

    #[test]
    fn dry_run_conflicts_with_quiet_and_verbose() {
        for output_flag in ["--quiet", "--verbose"] {
            let cli = Cli::try_parse_from(["dotrift", "apply", "--dry-run", output_flag]).unwrap();
            assert!(cli.resolve().is_err());
        }
    }
}
