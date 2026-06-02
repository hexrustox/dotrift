use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use miette::{Context, Result, miette};

use crate::{
    create_dir_err,
    path::{config_path, tmp_path},
    read_file_err, write_file_err,
};

use super::portal::PortalAnalysis;

pub fn toml_quote(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{}\"", escaped)
}

fn is_table_header(line: &str, name: &str) -> bool {
    line.trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .is_some_and(|inner| inner.trim() == name)
}

pub fn annotate_portal_key(content: &mut String, key: &str, annotation: &str) {
    let quoted = toml_quote(key);
    let mut in_portal = false;
    let mut found = false;
    let mut insert_at = 0;
    let mut offset = 0;

    for line in content.lines() {
        let line_end =
            offset + line.len() + content[offset + line.len()..].starts_with('\n') as usize;

        if !in_portal {
            if is_table_header(line.trim(), "portal") {
                in_portal = true;
            }
        } else if line.trim().starts_with('[') && !is_table_header(line.trim(), "portal") {
            break;
        } else {
            let trimmed = line.trim();
            if !trimmed.is_empty()
                && !trimmed.starts_with('#')
                && let Some(eq) = trimmed.find('=')
            {
                let lhs = trimmed[..eq].trim();
                if lhs == key || lhs == quoted {
                    found = true;
                    insert_at = offset;
                    break;
                }
            }
        }

        offset = line_end;
    }

    if found {
        content.insert_str(insert_at, &format!("{}\n", annotation));
    }
}

pub fn apply_config_changes(
    content: &str,
    analysis: &PortalAnalysis,
    target_dir: &Path,
) -> Option<String> {
    if analysis.missing.is_empty() && analysis.collisions.is_empty() {
        return None;
    }

    let mut new_content = if content.is_empty() {
        "[portal]".to_string()
    } else {
        content.to_string()
    };

    if !new_content.ends_with('\n') {
        new_content.push('\n');
    }

    if !new_content
        .lines()
        .any(|l| is_table_header(l.trim(), "portal"))
    {
        new_content.push_str("[portal]\n");
    }

    let mut auto_add_lines: Vec<String> = Vec::with_capacity(analysis.missing.len());
    let mut auto_add_keys: HashSet<String> = HashSet::with_capacity(analysis.missing.len());

    for (dest_rel, computed_target, warn) in &analysis.missing {
        let key_str = dest_rel.to_string_lossy().into_owned();
        auto_add_keys.insert(key_str.clone());

        if *warn {
            auto_add_lines.push(format!(
                "# WARNING: {} is outside target directory {}",
                computed_target.display(),
                target_dir.display()
            ));
        }
        auto_add_lines.push(format!(
            "{} = {}",
            toml_quote(&key_str),
            toml_quote(&computed_target.to_string_lossy())
        ));
    }

    for group in &analysis.collisions {
        for key in group {
            let others: Vec<_> = group.iter().filter(|k| *k != key).collect();
            let annotation = format!(
                "# CONFLICT with {}",
                others
                    .iter()
                    .map(|s| toml_quote(s))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if auto_add_keys.contains(key) {
                for (li, line) in auto_add_lines.iter_mut().enumerate() {
                    if line.trim().starts_with(&toml_quote(key))
                        || line.trim().starts_with(key.as_str())
                    {
                        auto_add_lines.insert(li, annotation.clone());
                        break;
                    }
                }
            } else {
                annotate_portal_key(&mut new_content, key, &annotation);
            }
        }
    }

    let insert_at = portal_insertion_point(&new_content);
    let mut insert = String::new();
    for line in &auto_add_lines {
        insert.push_str(line);
        insert.push('\n');
    }
    new_content.insert_str(insert_at, &insert);

    Some(new_content)
}

pub fn prepare_config(
    source_dir: &Path,
    analysis: &PortalAnalysis,
    target_dir: &Path,
) -> Result<Option<PathBuf>> {
    let config_path = config_path(source_dir);
    let content = if config_path.exists() {
        read_file_err!(fs::read_to_string(&config_path), config_path)?
    } else {
        String::new()
    };

    match apply_config_changes(&content, analysis, target_dir) {
        Some(modified) => {
            let temp_dir = tmp_path();
            create_dir_err!(fs::create_dir_all(&temp_dir), temp_dir)?;
            let temp_path = temp_dir.join(format!("{}.toml", std::process::id()));
            write_file_err!(fs::write(&temp_path, modified), temp_path)?;
            Ok(Some(temp_path))
        }
        None => Ok(None),
    }
}

pub fn portal_insertion_point(content: &str) -> usize {
    let mut in_portal = false;
    let mut insert_at = 0;
    let mut offset = 0;

    for line in content.lines() {
        let trimmed = line.trim();
        let line_content_end = offset + line.len();
        let has_newline = content[line_content_end..].starts_with('\n');
        let line_end = line_content_end + has_newline as usize;

        if !in_portal {
            if is_table_header(trimmed, "portal") {
                in_portal = true;
                insert_at = line_end;
            }
        } else if !trimmed.is_empty() && trimmed.starts_with('[') {
            break;
        } else if !trimmed.is_empty() && !trimmed.starts_with('#') && trimmed.contains('=') {
            insert_at = line_end;
        }

        offset = line_end;
    }

    insert_at
}

pub fn find_portal_cursor(content: &str) -> (u32, u32) {
    let offset = portal_insertion_point(content);
    let line = content[..offset].bytes().filter(|&b| b == b'\n').count() as u32 + 1;
    (line, 1)
}

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
                                "unclosed parameter in editor command argument: `{arg}`"
                            ));
                        }
                    }
                }
                match params.iter().find(|(n, _)| *n == name) {
                    Some((_, v)) => result.push_str(v),
                    None => {
                        return Err(miette!(
                            help = format!(
                                "valid parameters are: {}",
                                params
                                    .iter()
                                    .map(|(k, _)| format!("{{{k}}}"))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                            "unknown parameter `{name}` in editor command argument",
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use test_case::test_case;

    #[test_case("{file}", &[("file", "/path/to/config.toml")] => "/path/to/config.toml"; "replace_known")]
    #[test_case("+{row}", &[("row", "42")] => "+42"; "replace_with_prefix")]
    #[test_case("{col}", &[("col", "10")] => "10"; "replace_col")]
    #[test_case("-f", &[] => "-f"; "no_params")]
    #[test_case("{{file}}", &[] => "{file}"; "literal_braces")]
    #[test_case("}}", &[] => "}"; "literal_double_close")]
    #[test_case("{foo}", &[("file", "/f")] => panics "unknown parameter `foo`"; "unknown_param")]
    #[test_case("{file", &[("file", "/f")] => panics "unclosed parameter"; "unclosed_brace")]
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

    #[test_case(
        PortalAnalysis { missing: vec![], collisions: vec![] },
        "" => None
        ; "no_changes_returns_none"
    )]
    #[test_case(
        PortalAnalysis {
            missing: vec![("file".into(), "file".into(), false)],
            collisions: vec![],
        },
        "" => Some(r#"[portal]
"file" = "file"
"#.to_string())
        ; "missing_key_empty_content_creates_portal"
    )]
    #[test_case(
        PortalAnalysis {
            missing: vec![("file".into(), "file".into(), false)],
            collisions: vec![],
        },
        r#"[portal]
"existing" = "existing"
"# =>
        Some(r#"[portal]
"existing" = "existing"
"file" = "file"
"#.to_string())
        ; "missing_key_appended_to_existing_portal"
    )]
    #[test_case(
        PortalAnalysis {
            missing: vec![("file".into(), "/absolute/path".into(), true)],
            collisions: vec![],
        },
        r#"[portal]"# =>
        Some(r#"[portal]
# WARNING: /absolute/path is outside target directory /target
"file" = "/absolute/path"
"#.to_string())
        ; "missing_key_with_warning"
    )]
    #[test_case(
        PortalAnalysis {
            missing: vec![],
            collisions: vec![vec!["bare".into(), "quoted".into()]],
        },
        r#"[portal]
bare = "x"
"quoted" = "x"
"# =>
        Some(r#"[portal]
# CONFLICT with "quoted"
bare = "x"
# CONFLICT with "bare"
"quoted" = "x"
"#.to_string())
        ; "collision_annotates_existing_keys"
    )]
    #[test_case(
        PortalAnalysis {
            missing: vec![],
            collisions: vec![vec!["a".into(), "c".into()], vec!["b".into(), "d".into()]],
        },
        r#"[portal]
"a" = "x"
"b" = "y"
"c" = "x"
"d" = "y"
"# =>
        Some(r#"[portal]
# CONFLICT with "c"
"a" = "x"
# CONFLICT with "d"
"b" = "y"
# CONFLICT with "a"
"c" = "x"
# CONFLICT with "b"
"d" = "y"
"#.to_string())
        ; "multiple_collision_groups"
    )]
    #[test_case(
        PortalAnalysis {
            missing: vec![("new".into(), "new".into(), false)],
            collisions: vec![vec!["existing".into(), "new".into()]],
        },
        r#"[portal]
"existing" = "x"
"# =>
        Some(r#"[portal]
# CONFLICT with "new"
"existing" = "x"
# CONFLICT with "existing"
"new" = "new"
"#.to_string())
        ; "collision_on_auto_add_key"
    )]
    #[test_case(
        PortalAnalysis {
            missing: vec![("added".into(), "added".into(), false)],
            collisions: vec![],
        },
        r#"target-directory = "/foo"

[portal]
"# =>
        Some(r#"target-directory = "/foo"

[portal]
"added" = "added"
"#.to_string())
        ; "missing_key_preserves_other_sections"
    )]
    #[test_case(
        PortalAnalysis {
            missing: vec![("key".into(), "key".into(), false)],
            collisions: vec![],
        },
        "" => Some(r#"[portal]
"key" = "key"
"#.to_string())
        ; "empty_content"
    )]
    #[test_case(
        PortalAnalysis {
            missing: vec![],
            collisions: vec![vec!["a".into(), "b".into(), "c".into()]],
        },
        r#"[portal]
"a" = "x"
"b" = "x"
"c" = "x"
"# =>
        Some(r#"[portal]
# CONFLICT with "b", "c"
"a" = "x"
# CONFLICT with "a", "c"
"b" = "x"
# CONFLICT with "a", "b"
"c" = "x"
"#.to_string())
        ; "collision_annotates_three_way"
    )]
    #[test_case(
        PortalAnalysis {
            missing: vec![("key".into(), "key".into(), false)],
            collisions: vec![],
        },
        "[portal]\n\n# suffix\n" => Some(r#"[portal]
"key" = "key"

# suffix
"#.to_string())
        ; "missing_key_inserted_before_trailing_content"
    )]
    fn test_apply_config_changes(analysis: PortalAnalysis, content: &str) -> Option<String> {
        apply_config_changes(content, &analysis, Path::new("/target"))
    }
}
