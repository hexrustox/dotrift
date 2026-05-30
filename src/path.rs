use std::path::{Path, PathBuf};

use miette::{Result, miette};
use dirs::{config_dir, data_dir, state_dir};

pub const PKG_NAME: &str = env!("CARGO_PKG_NAME");

pub fn source_path() -> Result<PathBuf> {
    data_dir()
        .map(|d| d.join(PKG_NAME))
        .ok_or_else(|| miette!("Cannot determine data directory ($XDG_DATA_HOME or $HOME not set)"))
}

pub fn config_path(source_dir: &Path) -> PathBuf {
    source_dir.join(format!("{}.toml", PKG_NAME))
}

pub fn data_path(source_dir: &Path) -> PathBuf {
    source_dir.join(format!("{}_data.toml", PKG_NAME))
}

pub fn db_path() -> Result<PathBuf> {
    match state_dir() {
        Some(p) => Ok(p.join(PKG_NAME).join("db.sqlite")),
        None => data_dir()
            .map(|d| d.join(format!("{}.sqlite", PKG_NAME)))
            .ok_or_else(|| {
                miette!("Cannot determine state directory ($XDG_STATE_HOME or $HOME not set)")
            }),
    }
}

pub fn global_config_path() -> Result<PathBuf> {
    config_dir()
        .map(|d| d.join(PKG_NAME).join("config.toml"))
        .ok_or_else(|| {
            miette!("Cannot determine config directory ($XDG_CONFIG_HOME or $HOME not set)")
        })
}

pub fn tmp_path() -> PathBuf {
    std::env::temp_dir().join(PKG_NAME)
}
