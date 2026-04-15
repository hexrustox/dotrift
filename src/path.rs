use std::path::PathBuf;

use dirs::{data_dir, state_dir};

const PKG_NAME: &str = env!("CARGO_PKG_NAME");

pub fn source_path() -> PathBuf {
    data_dir().unwrap().join(PKG_NAME)
}

pub fn config_path(source_dir: PathBuf) -> PathBuf {
    source_dir.join(format!("{}.toml", PKG_NAME))
}

pub fn db_path() -> PathBuf {
    state_dir()
        .map(|p| p.join(PKG_NAME).join("manifest.sqlite"))
        .unwrap_or_else(|| data_dir().unwrap().join(format!("{}.sqlite", PKG_NAME)))
}
