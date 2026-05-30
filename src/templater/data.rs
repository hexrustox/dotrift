use std::{collections::HashMap, fs, path::Path};

use color_eyre::eyre::Context;
use serde::Deserialize;
use templater::value::Value;

use crate::path::data_path;

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct TemplateData {
    pub variable: HashMap<String, Value>,
    pub profile: HashMap<String, HashMap<String, Value>>,
}

impl TemplateData {
    pub fn read(source_dir: &Path) -> color_eyre::Result<Self> {
        let path = data_path(source_dir);
        if !path.is_file() {
            return Ok(Self::default());
        }
        let s = crate::read_file_err!(fs::read_to_string(&path), &path)?;
        crate::parse_err!(toml::from_str(&s), &path)
    }
}
