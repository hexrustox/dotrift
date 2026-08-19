pub mod capture;
pub mod cli;
pub mod commands;
pub mod config;
pub mod data;
pub mod hash;
pub mod managed;
pub mod state;
pub mod template;

use std::path::Path;

use miette::{Result, WrapErr, miette};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    Success = 0,
    Cancelled = 1,
    Skipped = 2,
}

pub(crate) fn ensure_absolute(path: &Path) -> Result<std::path::PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .map_err(|error| miette!(error))
            .wrap_err("cannot resolve the current directory")?
            .join(path))
    }
}
