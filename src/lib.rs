pub mod cli;
pub mod commands;
pub mod managed;
pub mod state;

use std::path::Path;

use miette::{Result, WrapErr, miette};

pub fn ensure_absolute(path: &Path) -> Result<std::path::PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .map_err(|error| miette!(error))
            .wrap_err("cannot resolve the current directory")?
            .join(path))
    }
}
