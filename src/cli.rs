use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "dotrift",
    version,
    about = "Declarative dotfile manager using TOML configuration"
)]
pub struct Cli {
    /// Override the source directory. Relative to current directory if path is not absolute. Default: $XDG_DATA_HOME/dotrift or ~/.local/share/dotrift.
    #[arg(short, long, name = "SOURCE_DIRECTORY")]
    pub source: Option<PathBuf>,

    /// Override the target directory. Relative to current directory if path is not absolute.
    #[arg(short, long, name = "TARGET_DIRECTORY")]
    pub target: Option<PathBuf>,

    /// Override the config file. Default: $XDG_CONFIG_HOME/dotrift/config.toml or ~/.config/dotrift/config.toml.
    #[arg(short, long, name = "CONFIG_FILE")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Args)]
pub struct ApplyFlags {
    /// Print planned operations without mutating the filesystem.
    #[arg(short, long)]
    pub dry_run: bool,

    /// Remove previously managed files that are no longer mapped in dotrift.toml.
    #[arg(short, long)]
    pub clean_up: bool,

    /// Recursively delete orphaned empty directories. Requires --clean-up.
    #[arg(short, long, requires = "clean_up")]
    pub prune_empty_dirs: bool,
}

#[derive(Args)]
pub struct UnapplyFlags {
    /// Print planned operations without mutating the filesystem.
    #[arg(short, long)]
    pub dry_run: bool,

    /// Recursively delete orphaned empty directories.
    #[arg(short, long)]
    pub prune_empty_dirs: bool,
}

#[derive(Args)]
pub struct AddFlags {
    /// Copy instead of moving the file or directory to the destination
    #[arg(short, long)]
    pub copy: bool,

    /// Remove any intermediate file and directories if they already exist
    #[arg(short, long)]
    pub force: bool,

    /// Whether to open dotrift.toml with your editor
    #[arg(short, long, name = "WHEN")]
    pub editor: Option<OpenEditor>,
}

#[derive(ValueEnum, Clone)]
pub enum OpenEditor {
    Always,
    Never,
}

#[derive(Subcommand)]
pub enum StatusSubcommand {
    /// List all managed files.
    List {
        /// Optional path to specify a file to operate on it only.
        file: Option<PathBuf>,
    },
    /// Clear status for all files.
    Clear {
        /// Optional path to specify a file to operate on it only.
        file: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialized the source directory.
    Init,

    /// Apply the dotfiles to the target directory.
    Apply(ApplyFlags),

    /// Reverses the apply process, removing managed files from the target directory.
    Unapply(UnapplyFlags),

    /// Adds existing file to source directory.
    Add {
        #[command(flatten)]
        flags: AddFlags,
        /// Path to existing file or directory.
        file: PathBuf,

        /// Path to move the file or directory to. Relative to the source directory if path is not absolute.
        destination: PathBuf,
    },

    /// Prints content differences between source and target file.
    Diff {
        /// Absolute path to specific file to check.
        file: PathBuf,

        /// Additional options passed to the diff command.
        #[arg(last = true)]
        extra_args: Vec<String>,
    },

    /// Reports management status of files in target directory.
    Status {
        #[command(subcommand)]
        command: StatusSubcommand,
    },
}
