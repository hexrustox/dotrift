use std::{fs, path::PathBuf};

use color_eyre::eyre::{Result, eyre};
use serde::Deserialize;

use crate::{
    error::{IoError, SerdeError},
    path::global_config_path,
};

pub fn expand_args(args: &[String], params: &[(&str, &str)]) -> Result<Vec<String>> {
    args.iter().map(|a| expand_arg(a, params)).collect()
}

pub fn expand_arg(arg: &str, params: &[(&str, &str)]) -> Result<String> {
    let mut result = String::with_capacity(arg.len());
    let mut chars = arg.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            if chars.peek() == Some(&'{') {
                chars.next();
                result.push('{');
            } else {
                let mut name = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(c) => name.push(c),
                        None => return Err(eyre!("Unclosed parameter in `{arg}`")),
                    }
                }
                match params.iter().find(|(n, _)| *n == name) {
                    Some((_, v)) => result.push_str(v),
                    None => return Err(eyre!("Unknown parameter `{{{name}}}`")),
                }
            }
        } else if c == '}' {
            if chars.peek() == Some(&'}') {
                chars.next();
                result.push('}');
            } else {
                result.push('}');
            }
        } else {
            result.push(c);
        }
    }
    Ok(result)
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "kebab-case", deny_unknown_fields)]
pub struct GlobalConfig {
    pub overwrite_identical: bool,
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

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    #[test_case("{file}", &[("file", "/path/to/config.toml")] => "/path/to/config.toml"; "replace_known")]
    #[test_case("+{row}", &[("row", "42")] => "+42"; "replace_with_prefix")]
    #[test_case("{col}", &[("col", "10")] => "10"; "replace_col")]
    #[test_case("-f", &[] => "-f"; "no_params")]
    #[test_case("{{file}}", &[] => "{file}"; "literal_braces")]
    #[test_case("}}", &[] => "}"; "literal_double_close")]
    #[test_case("{foo}", &[("file", "/f")] => panics "Unknown parameter `{foo}`"; "unknown_param")]
    #[test_case("{file", &[("file", "/f")] => panics "Unclosed parameter"; "unclosed_brace")]
    fn test_expand_arg(arg: &str, params: &[(&str, &str)]) -> String {
        expand_arg(arg, params).unwrap()
    }

    #[test_case(
        &["{file}".to_string(), "+{row}".to_string(), "+{col}".to_string()],
        &[("file", "/f"), ("row", "3"), ("col", "5")]
        => vec!["/f", "+3", "+5"]; "all_params"
    )]
    #[test_case(
        &["vim".to_string(), "+{row}".to_string(), "{file}".to_string()],
        &[("file", "/f"), ("row", "1")]
        => vec!["vim", "+1", "/f"]; "partial_pass_through"
    )]
    fn test_expand_args(args: &[String], params: &[(&str, &str)]) -> Vec<String> {
        expand_args(args, params).unwrap()
    }
}
