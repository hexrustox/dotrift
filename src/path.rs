use std::path::PathBuf;

use dirs::data_dir;

const PKG_NAME: &str = env!("CARGO_PKG_NAME");

pub fn source_path() -> PathBuf {
    data_dir().unwrap().join(PKG_NAME)
}

pub fn config_path(source_dir: PathBuf) -> PathBuf {
    source_dir.join(format!("{}.toml", PKG_NAME))
}
