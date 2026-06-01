use std::{
    collections::HashMap,
    fmt::Display,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};

use indexmap::IndexMap;
use miette::{Context, Result, miette};
use serde::Deserialize;

use crate::path::config_path;
use crate::templater::function::BuiltinFunctions;
use templater::{Template, Value};

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
    pub fn read(source_dir: &Path) -> Result<Self> {
        let path = config_path(source_dir);
        let s = crate::read_file_err!(fs::read_to_string(&path), &path)?;
        crate::parse_err!(toml::from_str(&s), &path)
    }

    pub fn read_templated(
        source_dir: &Path,
        variables: &HashMap<String, Value>,
        functions: &BuiltinFunctions,
    ) -> Result<Self> {
        let path = config_path(source_dir);
        let s = crate::read_file_err!(fs::read_to_string(&path), &path)?;

        let tmpl = crate::parse_template_err!(Template::from_bytes(s.into_bytes()), &path)?;
        let mut rendered = Vec::new();
        crate::render_template_err!(tmpl.render(&mut rendered, variables, functions), &path)?;
        let rendered_str = String::from_utf8_lossy(&rendered);

        Ok(crate::parse_err!(toml::from_str(&rendered_str), &path)?)
    }
}

pub type Portal = HashMap<String, PathBuf>;
pub type Rules = IndexMap<String, Rule>;

#[derive(Deserialize, Default, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct Rule {
    #[serde(rename = "type")]
    pub deploy_type: Option<DeployType>,
    pub mode: Option<FileMode>,
}

#[derive(Deserialize, Default, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum DeployType {
    #[default]
    Symlink,
    Copy,
    Tmpl,
}

impl Display for DeployType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeployType::Symlink => write!(f, "symlink"),
            DeployType::Copy => write!(f, "copy"),
            DeployType::Tmpl => write!(f, "tmpl"),
        }
    }
}

impl FromStr for DeployType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "symlink" => Ok(DeployType::Symlink),
            "copy" => Ok(DeployType::Copy),
            "tmpl" => Ok(DeployType::Tmpl),
            _ => Err(()),
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
            u16::from_str_radix(&s, 8).map_err(|_| format!("invalid octal mode: `{}`", s))?;

        if value > 0o777 {
            return Err(format!("mode `{}` exceeds 777", s));
        }

        Ok(FileMode(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    #[test_case("000", 0o0)]
    #[test_case("600", 0o600)]
    #[test_case("755", 0o755)]
    #[test_case("777", 0o777)]
    fn test_octal(str: &str, n: u16) {
        assert_eq!(FileMode::try_from(str.to_string()).unwrap(), FileMode(n));
    }

    #[test_case("abc")]
    #[test_case("800")]
    #[test_case("99")]
    #[test_case("1000")]
    fn test_invalid_octal(str: &str) {
        FileMode::try_from(str.to_string()).unwrap_err();
    }
}
