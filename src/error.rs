use std::{io, path::Path};

use color_eyre::eyre::{Context, Result};
use glob::PatternError;

pub trait IoError<T> {
    fn read_file_error(self, p: &Path) -> Result<T>;
    fn remove_file_error(self, p: &Path) -> Result<T>;
    fn copy_file_error(self, p1: &Path, p2: &Path) -> Result<T>;
    fn symlink_error(self, p: &Path) -> Result<T>;
    fn read_link_error(self, p: &Path) -> Result<T>;
    fn create_dir_error(self, p: &Path) -> Result<T>;
    fn remove_dir_error(self, p: &Path) -> Result<T>;
}

impl<T> IoError<T> for io::Result<T> {
    fn read_file_error(self, p: &Path) -> Result<T> {
        self.wrap_err_with(|| format!("Failed to read file `{}`", p.display()))
    }
    fn remove_file_error(self, p: &Path) -> Result<T> {
        self.wrap_err_with(|| format!("Failed to remove file `{}`", p.display()))
    }
    fn copy_file_error(self, p1: &Path, p2: &Path) -> Result<T> {
        self.wrap_err_with(|| format!("Failed to copy `{}` to `{}`", p1.display(), p2.display()))
    }
    fn symlink_error(self, p: &Path) -> Result<T> {
        self.wrap_err_with(|| format!("Failed to create symlink `{}`", p.display()))
    }
    fn read_link_error(self, p: &Path) -> Result<T> {
        self.wrap_err_with(|| format!("Failed to read symlink `{}`", p.display()))
    }
    fn create_dir_error(self, p: &Path) -> Result<T> {
        self.wrap_err_with(|| format!("Failed to create directory `{}`", p.display()))
    }
    fn remove_dir_error(self, p: &Path) -> Result<T> {
        self.wrap_err_with(|| format!("Failed to remove directory `{}`", p.display()))
    }
}

pub trait SerdeError<T> {
    fn parse_error(self, p: &Path) -> Result<T>;
}

impl<T, E: Send + Sync + std::error::Error + serde::de::Error + 'static> SerdeError<T>
    for Result<T, E>
{
    fn parse_error(self, p: &Path) -> Result<T> {
        self.wrap_err_with(|| format!("Failed to parse file `{}`", p.display()))
    }
}

pub trait EyreError<T> {
    fn wrap_as_db_error(self) -> Result<T>;
}

impl<T> EyreError<T> for Result<T> {
    fn wrap_as_db_error(self) -> Result<T> {
        self.wrap_err("SQLite database error")
    }
}

pub trait GlobError<T> {
    fn glob_error(self) -> Result<T>;
}

impl<T> GlobError<T> for Result<T, PatternError> {
    fn glob_error(self) -> Result<T> {
        self.wrap_err("Invalid glob pattern")
    }
}
