use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use miette::{Context, Result, miette};
use toml_edit::{DocumentMut, value};

use crate::{
    create_dir_err,
    path::{config_path, tmp_path},
    read_file_err, write_file_err,
};

use super::portal::PortalAnalysis;

fn annotate_key(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn apply_config_changes(
    mut doc: DocumentMut,
    analysis: &PortalAnalysis,
    target_dir: &Path,
) -> Result<String> {
    if !doc.contains_key("portal") {
        doc["portal"] = toml_edit::table();
    }

    let auto_add_keys: HashSet<String> = analysis
        .missing
        .iter()
        .map(|(dest_rel, _, _)| dest_rel.to_string_lossy().into_owned())
        .collect();

    for (dest_rel, computed_target, _) in &analysis.missing {
        let key_str = dest_rel.to_string_lossy().into_owned();
        let val_str = computed_target.to_string_lossy();

        let portal = doc["portal"].as_table_mut().unwrap();
        portal[&key_str] = value(val_str.as_ref());
    }

    for (dest_rel, computed_target, warn) in &analysis.missing {
        if *warn {
            let key_str = dest_rel.to_string_lossy().into_owned();
            let portal = doc["portal"].as_table_mut().unwrap();
            let (mut key_mut, _) = portal.get_key_value_mut(&key_str).unwrap();
            let existing = {
                let ld = key_mut.leaf_decor();
                ld.prefix()
                    .and_then(|r| r.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            key_mut.leaf_decor_mut().set_prefix(format!(
                "{existing}# WARNING: {} is outside target directory {}\n",
                computed_target.display(),
                target_dir.display()
            ));
        }
    }

    for group in &analysis.collisions {
        for key in group {
            let others: Vec<_> = group
                .iter()
                .filter(|k| *k != key)
                .map(|s| annotate_key(s))
                .collect();
            let annotation = format!("# CONFLICT with {}\n", others.join(", "));

            if auto_add_keys.contains(key) || {
                let portal = doc["portal"].as_table().unwrap();
                portal.contains_key(key)
            } {
                let portal = doc["portal"].as_table_mut().unwrap();
                let (mut key_mut, _) = portal.get_key_value_mut(key.as_str()).unwrap();
                let existing = {
                    let ld = key_mut.leaf_decor();
                    ld.prefix()
                        .and_then(|r| r.as_str())
                        .unwrap_or("")
                        .to_string()
                };
                key_mut
                    .leaf_decor_mut()
                    .set_prefix(format!("{existing}{annotation}"));
            }
        }
    }

    Ok(doc.to_string())
}

pub fn prepare_config(
    source_dir: &Path,
    analysis: &PortalAnalysis,
    target_dir: &Path,
) -> Result<Option<PathBuf>> {
    if analysis.missing.is_empty() && analysis.collisions.is_empty() {
        return Ok(None);
    }

    let config_path = config_path(source_dir);
    let content = if config_path.exists() {
        read_file_err!(fs::read_to_string(&config_path), config_path)?
    } else {
        String::new()
    };

    let doc = if content.is_empty() {
        DocumentMut::new()
    } else {
        crate::parse_err!(content.parse::<DocumentMut>(), &config_path)?
    };

    let modified = apply_config_changes(doc, analysis, target_dir)?;
    let temp_dir = tmp_path();
    create_dir_err!(fs::create_dir_all(&temp_dir), temp_dir)?;
    let temp_path = temp_dir.join(format!("{}.toml", std::process::id()));
    write_file_err!(fs::write(&temp_path, modified), temp_path)?;
    Ok(Some(temp_path))
}

pub fn find_portal_cursor(content: &str) -> (u32, u32) {
    let mut in_portal = false;
    let mut line = 1u32;
    for (i, l) in content.lines().enumerate() {
        let t = l.trim();
        if !in_portal {
            if t.strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
                .is_some_and(|inner| inner.trim() == "portal")
            {
                in_portal = true;
                line = i as u32 + 2;
            }
        } else if t.starts_with('[') {
            break;
        } else if !t.is_empty() && !t.starts_with('#') {
            line = i as u32 + 2;
        }
    }
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
        PortalAnalysis {
            missing: vec![("file".into(), "file".into(), false)],
            collisions: vec![],
        },
        "" => r#"[portal]
file = "file"
"#.to_string()
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
        r#"[portal]
"existing" = "existing"
file = "file"
"#.to_string()
        ; "missing_key_appended_to_existing_portal"
    )]
    #[test_case(
        PortalAnalysis {
            missing: vec![("file".into(), "/absolute/path".into(), true)],
            collisions: vec![],
        },
        r#"[portal]"# =>
        r#"[portal]
# WARNING: /absolute/path is outside target directory /target
file = "/absolute/path"
"#.to_string()
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
        r#"[portal]
# CONFLICT with "quoted"
bare = "x"
# CONFLICT with "bare"
"quoted" = "x"
"#.to_string()
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
        r#"[portal]
# CONFLICT with "c"
"a" = "x"
# CONFLICT with "d"
"b" = "y"
# CONFLICT with "a"
"c" = "x"
# CONFLICT with "b"
"d" = "y"
"#.to_string()
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
        r#"[portal]
# CONFLICT with "new"
"existing" = "x"
# CONFLICT with "existing"
new = "new"
"#.to_string()
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
        r#"target-directory = "/foo"

[portal]
added = "added"
"#.to_string()
        ; "missing_key_preserves_other_sections"
    )]
    #[test_case(
        PortalAnalysis {
            missing: vec![("key".into(), "key".into(), false)],
            collisions: vec![],
        },
        "" => r#"[portal]
key = "key"
"#.to_string()
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
        r#"[portal]
# CONFLICT with "b", "c"
"a" = "x"
# CONFLICT with "a", "c"
"b" = "x"
# CONFLICT with "a", "b"
"c" = "x"
"#.to_string()
        ; "collision_annotates_three_way"
    )]
    #[test_case(
        PortalAnalysis {
            missing: vec![("key".into(), "key".into(), false)],
            collisions: vec![],
        },
        r#"# prefix
[portal]

# suffix
"# => r#"# prefix
[portal]
key = "key"

# suffix
"#.to_string()
        ; "existing_portal_preserves_surrounding_comments"
    )]
    #[test_case(
        PortalAnalysis {
            missing: vec![("key".into(), "key".into(), false)],
            collisions: vec![],
        },
        r#"[map]
foo = "bar""# => r#"[map]
foo = "bar"

[portal]
key = "key"
"#.to_string()
        ; "no_portal_adds_portal_after_existing_sections"
    )]
    #[test_case(
        PortalAnalysis {
            missing: vec![("key".into(), "/outside/path".into(), true)],
            collisions: vec![vec!["key".into(), "other".into()]],
        },
        r#"[portal]
"other" = "x"
"# =>
        r#"[portal]
# CONFLICT with "key"
"other" = "x"
# WARNING: /outside/path is outside target directory /target
# CONFLICT with "other"
key = "/outside/path"
"#.to_string()
        ; "warn_and_collision_on_auto_add_key"
    )]
    fn test_apply_config_changes(analysis: PortalAnalysis, content: &str) -> String {
        let doc = content.parse().unwrap();
        apply_config_changes(doc, &analysis, Path::new("/target")).unwrap()
    }
}
