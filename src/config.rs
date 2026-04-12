use std::collections::HashMap;
use std::fs::read_to_string;
use std::path::PathBuf;

use color_eyre::eyre::Context;
use indexmap::IndexMap;
use serde::Deserialize;

use crate::path::config_path;

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

impl Config {
    pub fn read(source_dir: PathBuf) -> color_eyre::Result<Self> {
        let path = config_path(source_dir);
        let s = read_to_string(&path)
            .wrap_err(format!("Failed to read config file `{}`", path.display()))?;
        toml::from_str(&s).wrap_err("Failed to parse config file")
    }
}

pub type Portal = HashMap<String, PathBuf>;
pub type Rules = IndexMap<String, Rule>;

#[derive(Deserialize, Default, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    #[serde(default, rename = "type")]
    pub r#type: Option<DeployType>,
    #[serde(default)]
    pub mode: Option<FileMode>,
}

#[derive(Deserialize, Default, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum DeployType {
    #[default]
    Symlink,
    Copy,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(try_from = "String")]
pub struct FileMode(pub u16);

impl TryFrom<String> for FileMode {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        let value =
            u16::from_str_radix(&s, 8).map_err(|_| format!("Invalid octal mode: `{}`", s))?;

        if value > 0o777 {
            return Err(format!("Mode `{}` exceeds 777", s));
        }

        Ok(FileMode(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_valid_octal() {
        assert_eq!(
            FileMode::try_from("000".to_string()).unwrap(),
            FileMode(0o000)
        );
        assert_eq!(
            FileMode::try_from("600".to_string()).unwrap(),
            FileMode(0o600)
        );
        assert_eq!(
            FileMode::try_from("755".to_string()).unwrap(),
            FileMode(0o755)
        );
        assert_eq!(
            FileMode::try_from("777".to_string()).unwrap(),
            FileMode(0o777)
        );
    }

    #[test]
    fn mode_invalid_octal() {
        assert!(FileMode::try_from("abc".to_string()).is_err());
        assert!(FileMode::try_from("800".to_string()).is_err());
        assert!(FileMode::try_from("99".to_string()).is_err());
    }

    #[test]
    fn mode_exceeds_max() {
        assert!(FileMode::try_from("1000".to_string()).is_err());
        assert!(FileMode::try_from("7777".to_string()).is_err());
    }
}
