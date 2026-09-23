use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::platform::{Environment, ensure_absolute};

/// Deploy your dotfiles from a source directory to a target directory.
#[derive(Debug, Parser)]
#[command(about, version)]
pub struct Cli {
    /// Directory holding your dotfiles and `dotrift.toml`.
    ///
    /// Defaults to `$XDG_DATA_HOME/dotfiles` (or `$HOME/.local/share/dotfiles`).
    /// Relative paths resolve against the current directory.
    #[arg(short, long)]
    source: Option<PathBuf>,

    /// Directory to deploy into.
    ///
    /// Overrides `target-directory` in `dotrift.toml`.
    /// Relative paths resolve against the current directory.
    #[arg(short, long)]
    target: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Put your dotfiles in place in the target directory.
    Apply {
        /// Remove files dotrift no longer deploys.
        #[arg(long)]
        clean_up: bool,

        /// Also remove empty directories left behind (requires `--clean-up`).
        #[arg(long, requires = "clean_up")]
        prune_empty_dirs: bool,

        /// Show what would happen without changing anything.
        #[arg(long, conflicts_with_all = ["verbose", "quiet"])]
        dry_run: bool,

        /// Don't print the summary line.
        #[arg(long, conflicts_with = "verbose")]
        quiet: bool,

        /// Print one line per file as it is processed.
        #[arg(long, conflicts_with = "quiet")]
        verbose: bool,
    },

    /// Check the state of the files dotrift manages.
    Status,

    /// Create a new source directory with example config files.
    Init,

    /// Manage sets of template variables.
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProfileCommand {
    /// List all profiles, marking active ones.
    List,

    /// Activate a profile.
    Activate {
        /// Name of the profile.
        name: String,
    },

    /// Deactivate a profile.
    Deactivate {
        /// Name of the profile.
        name: String,
    },

    /// Print the final variable values used by templates.
    Show,
}

impl Cli {
    pub fn resolve(
        self,
        env: &Environment,
    ) -> miette::Result<(Option<PathBuf>, Option<PathBuf>, Command)> {
        let Cli {
            command,
            source,
            target,
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
                None => ensure_absolute(&env.default_source_dir()?)?,
            })
        };
        let target = target.map(|path| ensure_absolute(&path)).transpose()?;
        Ok((source, target, command))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    #[test_case(&["--prune-empty-dirs"]; "prune_empty_dirs_requires_clean_up")]
    #[test_case(&["--dry-run", "--verbose"]; "dry_run_conflicts_with_verbose")]
    #[test_case(&["--dry-run", "--quiet"]; "dry_run_conflicts_with_quiet")]
    #[test_case(&["--verbose", "--quiet"]; "verbose_conflicts_with_quiet")]
    fn apply_rejects_invalid_flag_combinations(flags: &[&str]) {
        assert!(Cli::try_parse_from([&["dotrift", "apply"], flags].concat()).is_err());
    }
}
