use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use glob::Pattern;
use miette::Result;
use normalize_path::NormalizePath;

use crate::{
    command::{
        apply::build_ignore,
        tree,
        util::{GLOB_OPTION, StripPrefixOrSelf, is_glob, resolve_portal_entries},
    },
    config::Config,
};

pub struct CompiledPortal {
    globs: Vec<Pattern>,
    literals: Vec<PathBuf>,
}

fn compile_portal(portal: &HashMap<String, PathBuf>) -> CompiledPortal {
    let mut globs = Vec::with_capacity(portal.len());
    let mut literals = Vec::with_capacity(portal.len());

    for key in portal.keys() {
        if is_glob(key) {
            if let Ok(p) = Pattern::new(key) {
                globs.push(p);
            }
        } else {
            literals.push(PathBuf::from(key));
        }
    }

    CompiledPortal { globs, literals }
}

fn portal_matches(dest_rel: &Path, compiled: &CompiledPortal) -> bool {
    compiled
        .globs
        .iter()
        .any(|p| p.matches_path_with(dest_rel, GLOB_OPTION))
        || compiled
            .literals
            .iter()
            .any(|path| dest_rel.ancestors().any(|a| a == path.as_path()))
}

pub struct PortalAnalysis {
    pub missing: Vec<(PathBuf, PathBuf, bool)>,
    pub collisions: Vec<Vec<String>>,
}

fn check_and_collect_collision(
    root: &mut tree::Node,
    collisions: &mut HashMap<String, HashSet<String>>,
    target_path: &Path,
    key: &str,
) {
    match root.check_entry(target_path, key.to_string()) {
        Ok(Some(existing)) => {
            let t = target_path.to_string_lossy().into_owned();
            collisions.entry(t.clone()).or_default().insert(existing);
            collisions.entry(t).or_default().insert(key.to_string());
        }
        Err(_) => {
            let t = target_path.to_string_lossy().into_owned();
            collisions
                .entry(t.clone())
                .or_default()
                .insert(key.to_string());
            for context_key in root.key_at(target_path) {
                collisions.entry(t.clone()).or_default().insert(context_key);
            }
        }
        _ => {}
    }
}

fn resolve_collisions(
    portal: &HashMap<String, PathBuf>,
    ignore: &[String],
    target_dir: &Path,
    source_dir: &Path,
) -> Result<(tree::Node, HashMap<String, HashSet<String>>)> {
    let ignore_matcher = build_ignore(ignore, target_dir)?;
    let mut root = tree::Node::default();
    let mut collisions: HashMap<String, HashSet<String>> = HashMap::new();

    resolve_portal_entries(
        source_dir,
        target_dir,
        portal,
        &ignore_matcher,
        true,
        |_, target_path, pattern_str| {
            check_and_collect_collision(&mut root, &mut collisions, &target_path, &pattern_str);
            Ok(())
        },
    )?;

    Ok((root, collisions))
}

fn check_new_entries(
    tree: &mut tree::Node,
    entries: &[(PathBuf, &Path)],
    target_dir: &Path,
) -> HashMap<String, HashSet<String>> {
    let mut new_collisions: HashMap<String, HashSet<String>> =
        HashMap::with_capacity(entries.len());

    for (computed_target, dest_rel) in entries {
        let target_path = target_dir.join(computed_target).normalize();
        let key = dest_rel.to_string_lossy().into_owned();
        check_and_collect_collision(tree, &mut new_collisions, &target_path, &key);
    }

    new_collisions
}

fn merge_collision_groups(
    existing: HashMap<String, HashSet<String>>,
    new: HashMap<String, HashSet<String>>,
) -> Vec<Vec<String>> {
    let mut merged = existing;
    for (target, keys) in new {
        merged.entry(target).or_default().extend(keys);
    }
    let result: Vec<Vec<String>> = merged
        .into_values()
        .map(|v| {
            #[allow(unused_mut)]
            let mut sorted: Vec<String> = v.into_iter().collect();
            #[cfg(test)]
            {
                sorted.sort();
            }
            sorted
        })
        .collect();
    result
}

pub fn analyze_portal(
    config: &Config,
    entries: &[(PathBuf, PathBuf)],
    target_dir: &Path,
    source_dir: &Path,
) -> Result<PortalAnalysis> {
    let (mut tree, existing_collisions) =
        resolve_collisions(&config.portal, &config.ignore, target_dir, source_dir)?;

    let compiled_portal = compile_portal(&config.portal);
    let mut precomputed: Vec<(PathBuf, &Path)> = Vec::with_capacity(entries.len());
    let mut missing = Vec::new();
    for (src, dest) in entries {
        let warn;
        let computed_target = if src.starts_with(target_dir) {
            warn = false;
            src.safe_strip_prefix(target_dir).to_path_buf()
        } else {
            warn = true;
            src.clone()
        };
        let dest_rel = dest.safe_strip_prefix(source_dir);

        precomputed.push((computed_target.clone(), dest_rel));

        if !portal_matches(dest_rel, &compiled_portal) {
            missing.push((dest_rel.to_path_buf(), computed_target, warn));
        }
    }

    let new_collisions = check_new_entries(&mut tree, &precomputed, target_dir);

    let collisions = merge_collision_groups(existing_collisions, new_collisions);

    Ok(PortalAnalysis {
        missing,
        collisions,
    })
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, path::Path};

    use super::*;
    use crate::{command::util::tests::setup_test, config::Config};
    use test_case::test_case;

    #[test_case("", "file" => true; "empty")]
    #[test_case(r#""" = """#, "file" => false; "match_root")]
    #[test_case(r#""dir/file" = """#, "dir/file" => false; "match_exact")]
    #[test_case(r#""dir" = """#, "dir/file" => false; "match_parent")]
    #[test_case(r#""a" = """#, "a/b/c" => false; "match_ancestor")]
    #[test_case(r#""a/b" = """#, "a/b/c" => false; "match_nested")]
    #[test_case(r#""a/b/c" = """#, "a/b/c" => false; "match_deep")]
    #[test_case(r#""*" = ".""#, "file" => false; "match_glob")]
    #[test_case(r#""dir/*" = """#, "dir/file" => false; "match_glob_parent")]
    #[test_case(r#""**/file" = """#, "a/b/file" => false; "match_glob_recursive")]
    #[test_case(r#""file1" = ""
"file2" = """#, "file1" => false; "match_first_portal")]
    #[test_case(r#""file1" = ""
"file2" = """#, "file2" => false; "match_second_portal")]
    #[test_case(r#""*.txt" = """#, "file" => true; "mismatch_glob")]
    #[test_case(r#""*.txt" = """#, "file.cfg" => true; "mismatch_glob_extension")]
    #[test_case(r#""file1" = """#, "file2" => true; "mismatch_literal")]
    #[test_case(r#""file" = """#, "file2" => true; "mismatch_prefix")]
    fn test_portal_matches(portal: &str, dest: &str) -> bool {
        let (_temp_dir, source_dir, _) = setup_test(portal, "", "", false);
        let config = Config::read(&source_dir).unwrap();
        let compiled = compile_portal(&config.portal);

        !portal_matches(Path::new(dest), &compiled)
    }

    #[test_case(
        r#""a" = "x""#,
        &["a"],
        ""
        => Vec::<(String, Vec<String>)>::new()
        ; "no_collision"
    )]
    #[test_case(
        r#""a" = "x"
"b" = "x""#,
        &["a", "b"],
        ""
        => vec![("x".into(), vec!["a".into(), "b".into()])]
        ; "same_target_two_literals"
    )]
    #[test_case(
        r#""subdir" = "out""#,
        &["subdir/f1", "subdir/f2"],
        ""
        => Vec::<(String, Vec<String>)>::new()
        ; "literal_dir"
    )]
    #[test_case(
        r#""subdir" = "out"
"flat" = "out/f1""#,
        &["subdir/f1", "flat"],
        ""
        => vec![("out/f1".into(), vec!["flat".into(), "subdir".into()])]
        ; "literal_dir_vs_flat_file"
    )]
    #[test_case(
        r#""*.txt" = """#,
        &["a.txt", "b.txt"],
        ""
        => Vec::<(String, Vec<String>)>::new()
        ; "glob"
    )]
    #[test_case(
        r#""ghost" = "x""#,
        &[],
        ""
        => Vec::<(String, Vec<String>)>::new()
        ; "missing_source"
    )]
    #[test_case(
        r#""a" = "x"
"b" = "x"
"c" = "y"
"d" = "y""#,
        &["a", "b", "c", "d"],
        ""
        => vec![("x".into(), vec!["a".into(), "b".into()]), ("y".into(), vec!["c".into(), "d".into()])]
        ; "multiple_groups"
    )]
    #[test_case(
        r#""a" = "x""#,
        &["a"],
        "\"x\""
        => Vec::<(String, Vec<String>)>::new()
        ; "ignore_skips"
    )]
    fn test_resolve_collisions(
        portal: &str,
        files: &[&str],
        ignore: &str,
    ) -> Vec<(String, Vec<String>)> {
        let (_temp_dir, source_dir, target_dir) = setup_test(portal, ignore, "", false);
        let config = Config::read(&source_dir).unwrap();

        for f in files {
            let path = source_dir.join(f);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, "").unwrap();
        }

        let (_, collisions) =
            resolve_collisions(&config.portal, &config.ignore, &target_dir, &source_dir).unwrap();

        let mut result = Vec::new();
        for (k, v) in collisions {
            let mut sorted: Vec<String> = v.into_iter().collect();
            sorted.sort();
            let rel = Path::new(&k)
                .safe_strip_prefix(&target_dir)
                .to_string_lossy()
                .into_owned();
            result.push((rel, sorted));
        }
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }

    #[test_case(
        &["x"],
        &[("x", "a")]
        => vec![("x".into(), vec!["a".to_string(), "x".to_string()])]
        ; "collision_with_marked"
    )]
    #[test_case(
        &["x"],
        &[("x", "a"), ("x", "b")]
        => vec![("x".into(), vec!["a".to_string(), "b".to_string(), "x".to_string()])]
        ; "multiple_new_same_target"
    )]
    #[test_case(
        &["x"],
        &[("y", "a")]
        => Vec::<(String, Vec<String>)>::new()
        ; "different_targets_no_collision"
    )]
    #[test_case(
        &[],
        &[("x", "a")]
        => Vec::<(String, Vec<String>)>::new()
        ; "empty_tree"
    )]
    #[test_case(
        &["dir/file"],
        &[("dir", "a"), ("dir", "b")]
        => vec![("dir".into(), vec!["a".into(), "b".into(), "dir/file".into()])]
        ; "files_collides_with_dir"
    )]
    #[test_case(
        &["file"],
        &[("file/a", "a"), ("file/b", "b")]
        => vec![("file/a".into(), vec!["a".into(), "file".into()]), ("file/b".into(), vec!["b".into(), "file".into()])]
        ; "dirs_collides_with_file"
    )]
    fn test_check_new_entries(
        tree_paths: &[&str],
        entries: &[(&str, &str)],
    ) -> Vec<(String, Vec<String>)> {
        let (_temp_dir, _source_dir, target_dir) = setup_test("", "", "", false);
        let mut tree = tree::Node::default();
        for p in tree_paths {
            tree.check_entry(&target_dir.join(p), p.to_string())
                .unwrap();
        }
        let precomputed: Vec<(PathBuf, &Path)> = entries
            .iter()
            .map(|(a, b)| {
                let src = target_dir.join(a);
                let computed = src.safe_strip_prefix(&target_dir).to_path_buf();
                (computed, Path::new(b))
            })
            .collect();
        let collisions = check_new_entries(&mut tree, &precomputed, &target_dir);
        let mut result = Vec::new();
        for (k, v) in collisions {
            let mut sorted: Vec<String> = v.into_iter().collect();
            sorted.sort();
            let rel = Path::new(&k)
                .safe_strip_prefix(&target_dir)
                .to_string_lossy()
                .into_owned();
            result.push((rel, sorted));
        }
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }

    #[test]
    fn test_merge_collision_groups() {
        let existing = HashMap::from([("/a".to_string(), HashSet::from(["k1".to_string()]))]);
        let new = HashMap::from([
            ("/a".to_string(), HashSet::from(["k1".to_string()])),
            ("/b".to_string(), HashSet::from(["k2".to_string()])),
        ]);
        let result = merge_collision_groups(existing, new);
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|keys| keys == &["k1"]));
        assert!(result.iter().any(|keys| keys == &["k2"]));
    }

    #[test_case(
        |s, _| { fs::write(s.join("a"), "").unwrap(); },
        r#""a" = "a""#,
        vec![("a", "a")]
        => (Vec::<(String, String)>::new(), vec![vec!["a".to_string()]])
        ; "single_literal_collision"
    )]
    #[test_case(
        |s, _| { fs::write(s.join("a"), "").unwrap(); },
        r#""*" = """#,
        vec![("a", "a")]
        => (Vec::<(String, String)>::new(), vec![vec!["*".to_string(), "a".to_string()]])
        ; "glob_collision"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("a"), "").unwrap();
            fs::write(s.join("b"), "").unwrap();
        },
        r#""a" = "x"
"b" = "x""#,
        vec![]
        => (Vec::<(String, String)>::new(), vec![vec!["a".to_string(), "b".to_string()]])
        ; "existing_portal_collision"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("a"), "").unwrap();
            fs::write(s.join("new"), "").unwrap();
        },
        r#""a" = "x""#,
        vec![("y", "new")]
        => (vec![("new".to_string(), "y".to_string())], Vec::<Vec<String>>::new())
        ; "missing_key"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("a"), "").unwrap();
            fs::write(s.join("new"), "").unwrap();
        },
        r#""a" = "x""#,
        vec![("x", "new")]
        => (
            vec![("new".to_string(), "x".to_string())],
            vec![vec!["a".to_string(), "new".to_string()]],
        )
        ; "both_missing_and_collision"
    )]
    #[test_case(
        |s, _| {
            fs::create_dir(s.join("dir")).unwrap();
            fs::write(s.join("dir/a"), "").unwrap();
            fs::write(s.join("b"), "").unwrap();
            fs::write(s.join("c"), "").unwrap();
        },
        r#""dir" = ""
"b" = "a"
        "#,
        vec![("a", "c")]
        => (
            vec![("c".to_string(), "a".to_string())],
            vec![vec!["b".to_string(), "c".to_string(), "dir".to_string()]],
        )
        ; "three_way_collision_at_same_target"
    )]
    fn test_analyze_portal(
        setup: impl FnOnce(&Path, &Path),
        portal: &str,
        entries: Vec<(&str, &str)>,
    ) -> (Vec<(String, String)>, Vec<Vec<String>>) {
        let (_temp_dir, source_dir, target_dir) = setup_test(portal, "", "", false);
        let config = Config::read(&source_dir).unwrap();

        setup(&source_dir, &target_dir);

        let entries: Vec<(PathBuf, PathBuf)> = entries
            .iter()
            .map(|(a, b)| (target_dir.join(a), source_dir.join(b)))
            .collect();

        let analysis = analyze_portal(&config, &entries, &target_dir, &source_dir).unwrap();
        let missing: Vec<(String, String)> = analysis
            .missing
            .iter()
            .map(|(d, c, _)| {
                (
                    d.to_string_lossy().into_owned(),
                    c.to_string_lossy().into_owned(),
                )
            })
            .collect();
        (missing, analysis.collisions)
    }
}
