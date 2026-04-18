use std::{
    collections::HashMap,
    fmt::Display,
    fs,
    path::{Path, PathBuf},
};

use indexmap::IndexMap;
use serde::Deserialize;

use crate::error::{IoError, SerdeError};
use crate::path::config_path;

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct Config {
    #[serde(rename = "target-directory")]
    pub target_dir: Option<PathBuf>,
    pub ignore: Vec<String>,
    pub portal: Portal,
    pub rule: Rules,
}

impl Config {
    pub fn read(source_dir: &Path) -> color_eyre::Result<Self> {
        let path = config_path(source_dir);
        let s = fs::read_to_string(&path).read_file_error(&path)?;
        toml::from_str(&s).parse_error(&path)
    }
}

pub type Portal = HashMap<String, PathBuf>;
pub type Rules = IndexMap<String, Rule>;

#[derive(Deserialize, Default, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct Rule {
    #[serde(rename = "type")]
    pub r#type: Option<DeployType>,
    pub mode: Option<FileMode>,
}

#[derive(Deserialize, Default, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum DeployType {
    #[default]
    Symlink,
    Copy,
}

impl Display for DeployType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeployType::Symlink => write!(f, "symlink"),
            DeployType::Copy => write!(f, "copy"),
        }
    }
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
