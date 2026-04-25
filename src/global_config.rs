use std::{fs, path::PathBuf};

use serde::Deserialize;

use crate::{
    error::{IoError, SerdeError},
    path::global_config_path,
};

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct GlobalConfig {
    pub overwrite_identical: bool,
    // TODO provide var to expand
    pub editor_command: Option<CommandConfig>,
}

impl GlobalConfig {
    pub fn read(path_override: Option<PathBuf>) -> color_eyre::Result<Self> {
        let specific = path_override.is_some();
        let path = path_override.unwrap_or(global_config_path());

        let result = fs::read_to_string(&path);
        if result.is_err() && !specific {
            return Ok(Self::default());
        }

        let content = result.read_file_error(&path)?;
        toml::from_str(&content).parse_error(&path)
    }
}

#[derive(Deserialize)]
pub struct CommandConfig {
    pub command: String,
    pub args: Vec<String>,
}
