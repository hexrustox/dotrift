use clap::{Args, Parser, Subcommand, ValueEnum};
use color_eyre::Result;
use std::path::PathBuf;

use crate::{command::to_absolute_path, path::source_path};

#[derive(Args)]
pub struct GlobalFlags {
    /// Override the source directory. Default: $XDG_DATA_HOME/dotrift or ~/.local/share/dotrift.
    #[arg(short, long, name = "SOURCE_DIRECTORY")]
    source: Option<PathBuf>,

    /// Override the target directory.
    #[arg(short, long, name = "TARGET_DIRECTORY")]
    target: Option<PathBuf>,

    /// Override the config file. Default: $XDG_CONFIG_HOME/dotrift/config.toml or ~/.config/dotrift/config.toml.
    #[arg(short, long, name = "CONFIG_FILE")]
    config: Option<PathBuf>,

    /// Enable verbose logging
    #[arg(short, long)]
    pub verbose: bool,
}

impl GlobalFlags {
    pub fn new(source: Option<PathBuf>, target: Option<PathBuf>, config: Option<PathBuf>) -> Self {
        Self {
            source,
            target,
            config,
            verbose: false,
        }
    }

    pub fn source(&self) -> Result<PathBuf> {
        self.source
            .as_ref()
            .map(|p| to_absolute_path(p))
            .unwrap_or_else(source_path)
    }

    pub fn target(&self) -> Result<Option<PathBuf>> {
        self.target
            .as_ref()
            .map(|p| to_absolute_path(p))
            .transpose()
    }

    pub fn config(&self) -> Result<Option<PathBuf>> {
        self.config
            .as_ref()
            .map(|p| to_absolute_path(p))
            .transpose()
    }
}

#[derive(Parser)]
#[command(name = "dotrift", version, about)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalFlags,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Args, Clone, Copy)]
pub struct ApplyFlags {
    /// Print planned operations without touching the filesystem or database.
    #[arg(short, long)]
    pub dry_run: bool,

    /// Remove previously managed files no longer mapped in `dotrift.toml`.
    #[arg(short, long)]
    pub clean_up: bool,

    /// Recursively delete orphaned empty directories. Requires `--clean-up`.
    #[arg(short, long, requires = "clean_up")]
    pub prune_empty_dirs: bool,
}

#[derive(Args)]
pub struct UnapplyFlags {
    /// Print planned operations without touching the filesystem or database.
    #[arg(short, long)]
    pub dry_run: bool,

    /// Recursively delete orphaned empty directories.
    #[arg(short, long)]
    pub prune_empty_dirs: bool,
}

#[derive(Args, Clone, Copy)]
pub struct AddFlags {
    /// Copy instead of moving. Implicit in re-import mode.
    #[arg(short, long)]
    pub copy: bool,

    /// Remove obstructions blocking the move/copy.
    #[arg(short, long)]
    pub force: bool,

    /// When to open `dotrift.toml` in editor. Default: auto (open if Missing key or Target collision is detected).
    #[arg(short, long, name = "WHEN")]
    pub editor: Option<OpenEditor>,

    /// Do not modify `dotrift.toml` (skip auto-add and collision annotations). Editor may still open for manual configuration.
    #[arg(short = 'n', long)]
    pub no_modify: bool,
}

#[derive(ValueEnum, Clone, Copy)]
pub enum OpenEditor {
    Always,
    Never,
}

#[derive(Subcommand)]
pub enum StatusSubcommand {
    /// List all managed files, or check a specific file.
    List {
        /// Optional path to check a specific file.
        file: Option<PathBuf>,
    },
    /// Clear status for a specific file, or all files if omitted.
    Clear {
        /// Optional path to clear a specific file.
        file: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialized the source directory.
    Init,

    /// Evaluate `dotrift.toml` and apply the defined state to the target filesystem.
    Apply(ApplyFlags),

    /// Reverse the apply process, removing managed files from the target.
    Unapply(UnapplyFlags),

    /// Add existing file to source directory.
    Add {
        #[command(flatten)]
        flags: AddFlags,
        /// Path to existing file, directory, or symlink. If relative, resolved against cwd.
        path: PathBuf,

        /// Optional path in source directory. When omitted, re-import mode (destination derived from DB).
        destination: Option<PathBuf>,
    },

    /// Show a side-by-side diff between a managed file and its source.
    Diff {
        /// Path to a managed file to diff. If relative, resolved against cwd.
        path: PathBuf,
    },

    /// Report management status of the target filesystem.
    Status {
        #[command(subcommand)]
        command: StatusSubcommand,
    },
}
