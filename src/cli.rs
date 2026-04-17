use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "dotrift",
    version,
    about = "Declarative dotfile manager using TOML configuration."
)]
pub struct Cli {
    /// Path to the source directory containing dotrift.toml and dotfiles. Default: $XDG_DATA_HOME/dotrift or ~/.local/share/dotrift.
    #[arg(short = 's', long)]
    pub source: Option<PathBuf>,

    /// Override the target directory.
    #[arg(short = 't', long)]
    pub target: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Args)]
pub struct ApplyFlags {
    /// Print planned operations without mutating the filesystem.
    #[arg(short = 'd', long)]
    pub dry_run: bool,

    /// Remove previously managed files that are no longer mapped in dotrift.toml.
    #[arg(short = 'c', long)]
    pub clean_up: bool,

    /// Recursively delete orphaned empty directories. Requires --clean-up.
    #[arg(short = 'p', long, requires = "clean_up")]
    pub prune_empty_dirs: bool,
}

#[derive(Args)]
pub struct UnapplyFlags {
    /// Print planned operations without mutating the filesystem.
    #[arg(short = 'd', long)]
    pub dry_run: bool,

    /// Recursively delete orphaned empty directories.
    #[arg(short = 'p', long, requires = "clean_up")]
    pub prune_empty_dirs: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Evaluates dotrift.toml and applies the defined state to the target directory.
    Apply(ApplyFlags),

    /// Reverses the apply process, removing managed files from the target.
    Unapply(UnapplyFlags),

    /// Adds existing target file to source directory.
    Add {
        /// Absolute path to existing file on disk.
        target_file: PathBuf,

        /// Desired location relative to source directory.
        source_relative: String,
    },

    /// Prints content differences between source and target file.
    Diff {
        /// Absolute path to specific file to check.
        target_file: PathBuf,

        /// Additional options passed to the diff command.
        #[arg(last = true)]
        extra_args: Vec<String>,
    },

    /// Reports management status of target filesystem.
    Status {
        /// Optional absolute path to specific file (omitted: lists all managed files).
        target_file: Option<PathBuf>,
    },
}
