use std::path::{Path, PathBuf};

use dirs::{config_dir, data_dir, state_dir};

const PKG_NAME: &str = env!("CARGO_PKG_NAME");

pub fn source_path() -> PathBuf {
    data_dir().unwrap().join(PKG_NAME)
}

pub fn config_path(source_dir: &Path) -> PathBuf {
    source_dir.join(format!("{}.toml", PKG_NAME))
}

pub fn db_path() -> PathBuf {
    state_dir()
        .map(|p| p.join(PKG_NAME).join("state.sqlite"))
        .unwrap_or_else(|| data_dir().unwrap().join(format!("{}.sqlite", PKG_NAME)))
}

pub fn global_config_path() -> PathBuf {
    config_dir().unwrap().join(PKG_NAME).join("config.toml")
}
