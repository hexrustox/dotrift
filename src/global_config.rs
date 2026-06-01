use std::{fs, path::PathBuf};

use miette::{Context, Result, miette};
use serde::Deserialize;

use crate::path::global_config_path;

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct GlobalConfig {
    pub overwrite_identical: bool,
    pub editor_command: Option<CommandConfig>,
}

impl GlobalConfig {
    pub fn read(path_override: Option<PathBuf>) -> Result<Self> {
        let specific = path_override.is_some();
        let path = match path_override {
            Some(p) => p,
            None => global_config_path()?,
        };

        if !path.is_file() && !specific {
            return Ok(Self::default());
        }
        let s = crate::read_file_err!(fs::read_to_string(&path), &path)?;
        crate::parse_err!(toml::from_str(&s), &path)
    }
}

#[derive(Deserialize)]
pub struct CommandConfig {
    pub command: String,
    pub args: Vec<String>,
}
