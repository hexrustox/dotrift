use std::fs;

use color_eyre::eyre::{Context, eyre};

use crate::{cli::GlobalFlags, command::util::PathLiteral, path::config_path};

pub fn run(global_flags: GlobalFlags) -> color_eyre::Result<()> {
    let source_dir = global_flags.source()?;

    let path = config_path(&source_dir);

    if !path.path_exists() {
        let parent = path.parent().unwrap();
        crate::create_dir_err!(fs::create_dir_all(parent), parent)?;
        fs::write(&path, include_bytes!("./template.toml"))
            .wrap_err_with(|| format!("Failed to write file `{}`", path.display()))?;

        eprintln!("Initialized at `{}`", path.display());
        Ok(())
    } else {
        Err(eyre!("`{}` already initialized", path.display()))
    }
}
