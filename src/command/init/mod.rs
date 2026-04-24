// TODO print more
use std::fs;

use color_eyre::eyre::{Context, eyre};

use crate::{cli::GlobalFlags, command::util::PathLiteral, error::IoError, path::config_path};

pub fn run(global_flags: GlobalFlags) -> color_eyre::Result<()> {
    let source_dir = global_flags.source()?;

    let path = config_path(&source_dir);

    if !path.literal_exists() {
        let parent = path.parent().unwrap();
        fs::create_dir_all(parent).create_dir_error(parent)?;
        fs::write(&path, include_bytes!("./template.toml"))
            .wrap_err_with(|| format!("Failed to write file `{}`", path.display()))?;

        Ok(())
    } else {
        Err(eyre!("Source directory already initialized"))
    }
}
