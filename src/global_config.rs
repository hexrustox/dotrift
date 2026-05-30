use std::{fs, io::ErrorKind, path::PathBuf};

use miette::{Context, Result, miette};
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
                            return Err(miette!(
                                "Unclosed parameter in editor command argument: `{arg}`"
                            ));
                        }
                    }
                }
                match params.iter().find(|(n, _)| *n == name) {
                    Some((_, v)) => result.push_str(v),
                    None => {
                        return Err(miette!(
                            help = format!(
                                "Valid parameters are: {}",
                                params
                                    .iter()
                                    .map(|(k, _)| format!("{{{k}}}"))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                            "Unknown parameter `{name}` in editor command argument",
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

pub fn portal_insertion_point(content: &str) -> usize {
    let bytes = content.as_bytes();
    let mut in_portal = false;
    let mut insert_at = 0;
    let mut i = 0;

    while i < bytes.len() {
        let line_start = i;
        while i < bytes.len() && bytes[i] != b'\n' {
            i += 1;
        }
        let line_end = if i < bytes.len() { i + 1 } else { i };
        let trimmed = content[line_start..i].trim();

        if !in_portal {
            if trimmed == "[portal]" {
                in_portal = true;
                insert_at = line_end;
            }
        } else if !trimmed.is_empty() && trimmed.starts_with('[') {
            break;
        } else if !trimmed.is_empty() && !trimmed.starts_with('#') && trimmed.contains('=') {
            insert_at = line_end;
        }

        i = line_end;
    }

    insert_at
}

pub fn find_portal_cursor(content: &str) -> (u32, u32) {
    let offset = portal_insertion_point(content);
    let line = content[..offset].bytes().filter(|&b| b == b'\n').count() as u32 + 1;
    (line, 1)
}

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

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == ErrorKind::NotFound && !specific => return Ok(Self::default()),
            Err(e) => {
                return Err(e).map_err(|e| miette!(e)).wrap_err_with(|| {
                    format!("Failed to read global config `{}`", path.display())
                });
            }
        };
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
    #[test_case("{foo}", &[("file", "/f")] => panics "Unknown parameter `foo`"; "unknown_param")]
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
