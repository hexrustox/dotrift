use std::{fs, path::PathBuf};

use color_eyre::eyre::{Context, Result, eyre};
use serde::Deserialize;

use crate::path::global_config_path;

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
                        None => {
                            return Err(eyre!(
                                "Unclosed parameter in editor command argument: `{arg}`"
                            ));
                        }
                    }
                }
                match params.iter().find(|(n, _)| *n == name) {
                    Some((_, v)) => result.push_str(v),
                    None => {
                        return Err(eyre!(
                            "Unknown parameter `{{{name}}}` in editor command argument"
                        ));
                    }
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

pub fn find_portal_cursor(content: &str) -> (u32, u32) {
    let mut in_portal = false;
    let mut last = 1;

    for (i, line) in content.lines().enumerate() {
        let t = line.trim();
        if t == "[portal]" {
            in_portal = true;
            last = i as u32 + 2;
        } else if in_portal {
            if t.starts_with('[') {
                break;
            }
            if !t.is_empty() && !t.starts_with('#') {
                last = i as u32 + 2;
            }
        }
    }
    (last, 1)
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
        let path = match path_override {
            Some(p) => p,
            None => global_config_path()?,
        };

        let result = fs::read_to_string(&path);
        if result.is_err() && !specific {
            return Ok(Self::default());
        }

        let content = crate::read_file_err!(result, &path)?;
        crate::parse_err!(toml::from_str(&content), &path)
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

    #[test_case("" => (1, 1); "empty")]
    #[test_case("[rule]\nk = v\n" => (1, 1); "no_portal")]
    #[test_case("[portal]\n" => (2, 1); "portal_empty")]
    #[test_case("[portal]\nk = v\n" => (3, 1); "portal_one_entry")]
    #[test_case("[portal]\na = b\nc = d\n" => (4, 1); "portal_multi_entry")]
    #[test_case("[portal]\n# note\na = b\n" => (4, 1); "portal_with_comment")]
    #[test_case("[portal]\n\n\na = b\n" => (5, 1); "portal_with_blanks")]
    #[test_case("[portal]\na = b\n\n[rule]\n" => (3, 1); "portal_before_other_table")]
    #[test_case(r#"[portal]
"a" = b
"# => (3, 1); "portal_quoted_key")]
    fn test_find_portal_cursor(content: &str) -> (u32, u32) {
        find_portal_cursor(content)
    }
}
