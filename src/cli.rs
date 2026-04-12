use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "dotrift",
    version,
    about = "Declarative dotfile manager using TOML configuration."
)]
pub struct Cli {
    /// Path to the source directory containing dotrift.toml and dotfiles. Default: ~/.local/share/dotrift.
    #[arg(short = 's', long, default_value = "~/.local/share/dotrift")]
    pub source: PathBuf,

    /// Override the target directory.
    #[arg(short = 't', long)]
    pub target: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

/// Common options for apply and unapply commands.
#[derive(Args, Clone, Copy, Debug)]
pub struct CommonFlags {
    /// Print planned operations without touching the filesystem or database.
    #[arg(short = 'd', long)]
    pub dry_run: bool,

    /// Remove previously managed files no longer mapped in dotrift.toml.
    #[arg(short = 'c', long)]
    pub clean_up: bool,

    /// Recursively delete orphaned empty directories. Requires --clean-up.
    #[arg(short = 'p', long, requires = "clean_up")]
    pub prune_empty_dirs: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Evaluates dotrift.toml and applies the defined state to the target filesystem.
    Apply(CommonFlags),

    /// Reverses the apply process, removing managed files from the target.
    Unapply(CommonFlags),

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
