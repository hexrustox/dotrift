use std::{fs, path::Path};

use serde::Deserialize;

use crate::error::{IoError, SerdeError};

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct GlobalConfig {
    pub overwrite_identical: bool,
    // TODO provide var to expand
    pub editor_command: Option<CommandConfig>,
}

impl GlobalConfig {
    pub fn read(path: &Path) -> color_eyre::Result<Self> {
        let s = fs::read_to_string(path).read_file_error(path)?;
        toml::from_str(&s).parse_error(path)
    }
}

#[derive(Deserialize)]
pub struct CommandConfig {
    pub command: String,
    pub args: Vec<String>,
}
