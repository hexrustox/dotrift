use std::collections::HashMap;
use std::path::PathBuf;

use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(rename = "target-dir")]
    pub target_dir: Option<PathBuf>,
    #[serde(default)]
    pub ignore: Vec<String>,
    #[serde(default)]
    pub portal: Portal,
    #[serde(default)]
    pub rule: Rules,
}

pub type Portal = HashMap<String, String>;
pub type Rules = IndexMap<String, Rule>;

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    #[serde(default, rename = "type")]
    pub r#type: DeployType,
    #[serde(default)]
    pub mode: Option<Mode>,
}

#[derive(Deserialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DeployType {
    #[default]
    Symlink,
    Copy,
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct Mode(pub u16);

impl TryFrom<String> for Mode {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        let value =
            u16::from_str_radix(&s, 8).map_err(|_| format!("Invalid octal mode: `{}`", s))?;

        if value > 0o777 {
            return Err(format!("Mode `{}` exceeds 777", s));
        }

        Ok(Mode(value))
    }
}
