// TODO add doc comments
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use miette::Result;

use crate::platform::{Environment, ensure_absolute};

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
        #[arg(long, requires = "clean_up")]
        prune_empty_dirs: bool,
        #[arg(long, conflicts_with_all = ["verbose", "quiet"])]
        dry_run: bool,
        #[arg(long, conflicts_with = "verbose")]
        quiet: bool,
        #[arg(long, conflicts_with = "quiet")]
        verbose: bool,
    },
    Status,
    Init,
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
    pub fn resolve(self, env: &Environment) -> Result<(Option<PathBuf>, Option<PathBuf>, Command)> {
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

    #[test]
    fn init_accepts_the_global_target_flag() {
        let cli = Cli::try_parse_from(["dotrift", "-t", "/tmp/file1", "init"])
            .expect("-t is accepted by every subcommand");
        assert!(matches!(cli.command, Command::Init));
    }

    #[test]
    fn init_resolves_the_default_source_directory() {
        let env = Environment::default();
        let cli = Cli::try_parse_from(["dotrift", "init"]).unwrap();
        let (source, _, command) = cli.resolve(&env).unwrap();
        assert!(matches!(command, Command::Init));
        assert_eq!(source.unwrap(), env.default_source_dir().unwrap());
    }

    #[test]
    fn init_resolves_the_source_override() {
        let env = Environment::default();
        let cli = Cli::try_parse_from(["dotrift", "-s", "/tmp/dir1", "init"]).unwrap();
        let (source, _, command) = cli.resolve(&env).unwrap();
        assert!(matches!(command, Command::Init));
        assert_eq!(source.unwrap(), PathBuf::from("/tmp/dir1"));
    }

    #[test_case(&["dotrift", "init", "file1"]; "rejects_a_positional_argument")]
    #[test_case(&["dotrift", "init", "--clean-up"]; "rejects_an_apply_flag")]
    #[test_case(&["dotrift", "init", "--source"]; "rejects_a_flag_missing_its_value")]
    fn init_rejects_extra_arguments(argv: &[&str]) {
        assert!(Cli::try_parse_from(argv).is_err());
    }
}
