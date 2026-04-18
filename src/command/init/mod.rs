use std::{fs, path::PathBuf};

use color_eyre::eyre::{Context, eyre};

use crate::{error::IoError, path::config_path};

pub fn run(source_dir: PathBuf) -> color_eyre::Result<()> {
    let path = config_path(&source_dir);
    if matches!(path.try_exists(), Ok(false)) {
        let parent = path.parent().unwrap();
        fs::create_dir_all(parent).create_dir_error(parent)?;
        fs::write(&path, include_bytes!("./template.toml"))
            .wrap_err_with(|| format!("Failed to write file `{}`.", path.display()))?;

        Ok(())
    } else {
        Err(eyre!("Source directory already initialized"))
    }
}
