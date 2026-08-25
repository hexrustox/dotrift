pub mod capture;
pub mod cli;
pub mod commands;
pub mod config;
pub mod data;
pub mod hash;
pub mod managed;
pub mod state;
pub mod template;

use std::{
    fs,
    path::{Path, PathBuf},
};

use miette::{Result, WrapErr, miette};
use normalize_path::NormalizePath;

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

pub(crate) fn ensure_source_dir(path: &Path) -> Result<()> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(miette!(
            "source directory `{}` is not a directory",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(miette!(
            "source directory `{}` does not exist",
            path.display()
        )),
        Err(error) => Err(miette!(error).wrap_err(format!(
            "cannot access source directory `{}`",
            path.display()
        ))),
    }
}

pub(crate) fn prettify_path(path: &Path) -> PathBuf {
    let normalized = path.normalize();
    if let Some(home) = dirs::home_dir() {
        let home = home.normalize();
        if let Ok(stripped) = normalized.strip_prefix(&home) {
            if stripped.as_os_str().is_empty() {
                return PathBuf::from("~");
            }
            let mut result = PathBuf::from("~");
            result.extend(stripped.iter());
            return result;
        }
    }
    normalized
}
