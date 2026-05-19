use std::{
    collections::{HashMap, HashSet},
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Command,
};

use color_eyre::{
    Section,
    eyre::{Context, Result, eyre},
};
use glob::Pattern;
use normalize_path::NormalizePath;

use crate::{
    cli::{AddFlags, GlobalFlags, OpenEditor},
    command::{
        apply::{build_ignore, is_ignored},
        to_absolute_path, tree,
        util::{
            GLOB_OPTION, PathLiteral, SafeStripPrefix, copy_recursive, is_glob, resolve_target,
            strip_prefix_filter_glob, walk_files,
        },
    },
    config::Config,
    copy_file_err, create_dir_err,
    db::Db,
    global_config::{GlobalConfig, expand_args, find_portal_cursor, portal_insertion_point},
    output,
    path::{PKG_NAME, config_path, tmp_path},
    read_file_err, write_file_err,
};

pub fn run(
    global_flags: GlobalFlags,
    path: PathBuf,
    destination: Option<PathBuf>,
    flags: AddFlags,
    db_path: &Path,
) -> Result<()> {
    let source_dir = global_flags.source()?;
    let path = to_absolute_path(&path)?;

    let reimport = destination.is_none();

    let reimport_dir = path.path_is_dir() && reimport;

    let entries: Vec<(PathBuf, PathBuf)> = if reimport_dir {
        let db = Db::init(db_path)?;
        let mut entries = Vec::new();
        for entry in walk_files(&path) {
            let p = entry.path();
            match db.get_entry(p)? {
                Some(db_entry) => {
                    let dest = db_entry.source_path;
                    if !dest.starts_with(&source_dir) {
                        output::print_warn(format!(
                            "`{}` is outside source directory, skipping",
                            p.display()
                        ));
                        continue;
                    }
                    if dest.path_exists() && !flags.force {
                        output::print_warn(format!(
                            "`{} already exists`, skipping",
                            dest.display()
                        ));
                        continue;
                    }
                    entries.push((p.to_path_buf(), dest));
                }
                None => {
                    output::print_warn(format!("`{}` is not in database, skipping", p.display()));
                }
            }
        }
        entries
    } else {
        let dest = if let Some(d) = destination {
            if d.is_absolute() {
                d
            } else {
                source_dir.join(d).normalize()
            }
        } else {
            let db = Db::init(db_path)?;
            match db.get_entry(&path)? {
                Some(entry) => entry.source_path,
                None => {
                    return Err(
                        eyre!("Path `{}` not found in database", path.display()).note(format!(
                            "Database records files that were deployed with `{} apply`",
                            PKG_NAME
                        )),
                    );
                }
            }
        };
        if !dest.starts_with(&source_dir) {
            return Err(eyre!(
                "Destination `{}` must be inside source directory `{}`",
                dest.display(),
                source_dir.display()
            ));
        }
        vec![(path, dest)]
    };

    if !reimport_dir {
        let dest = &entries[0].1;
        if dest.path_exists() && !flags.force {
            return Err(eyre!("`{}` already exists", dest.display())
                .suggestion("Use --force to overwrite the existing file, or remove it manually"));
        }
    }

    let verbose = global_flags.verbose;
    for (src, dest) in &entries {
        if flags.force {
            remove_obstructions(dest, verbose)?;
        }
        if let Some(parent) = dest.parent() {
            crate::create_dir_err!(fs::create_dir_all(parent), parent)?;
        }
        if reimport || flags.copy {
            copy_recursive(src, dest)
        } else {
            fs::rename(src, dest).wrap_err_with(|| {
                format!("Failed to move `{}` to `{}`", src.display(), dest.display())
            })
        }?;
        if verbose {
            output::print_added(src, dest);
        }
    }

    let config = Config::read(&source_dir)?;
    let target_override = global_flags.target()?;
    let target_dir = resolve_target(&source_dir, target_override, &config)?;

    let analysis = analyze_portal(&config, &entries, &target_dir, &source_dir)?;

    let should_open = match flags.editor {
        Some(OpenEditor::Always) => true,
        Some(OpenEditor::Never) => false,
        None => !analysis.missing.is_empty() || !analysis.collisions.is_empty(),
    };

    #[cfg(test)]
    {
        tests::OPEN_EDITOR.set(should_open);
    }

    let config_override = global_flags.config()?;
    let temp_config = if !flags.no_modify && should_open {
        prepare_config(&source_dir, &analysis, &target_dir)?
    } else {
        None
    };

    if should_open {
        let config_path = config_path(&source_dir);
        if let Some(ref temp) = temp_config {
            let before = fs::metadata(temp).ok().and_then(|m| m.modified().ok());
            launch_editor(temp, config_override)?;
            let after = fs::metadata(temp).ok().and_then(|m| m.modified().ok());
            if before.is_some_and(|b| after.is_some_and(|a| a != b)) {
                copy_file_err!(fs::copy(temp, &config_path), temp, config_path)?;
            }
            let _ = fs::remove_file(temp);
        } else {
            launch_editor(&config_path, config_override)?;
        }
    }

    Ok(())
}

struct CompiledPortal {
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

    for (pattern, target_rel) in portal {
        let pattern_normalized = Path::new(pattern).normalize();
        let pattern_str = pattern_normalized.to_string_lossy().into_owned();
        let target_rel_normalized = target_rel.normalize();

        if is_glob(&pattern_str) {
            let prefix = strip_prefix_filter_glob(&pattern_str);
            let full_pattern = source_dir.join(&pattern_str);
            let full_pattern_str = full_pattern.to_string_lossy();

            let Ok(paths) = crate::glob_err!(
                glob::glob_with(&full_pattern_str, GLOB_OPTION),
                &full_pattern_str
            ) else {
                continue;
            };
            for source_path in paths.flatten() {
                if source_path.path_is_dir() {
                    continue;
                }
                let source_rel = source_path.safe_strip_prefix(source_dir);
                let stripped = if prefix.is_empty() {
                    source_rel.to_path_buf()
                } else {
                    source_rel.safe_strip_prefix(&prefix).to_path_buf()
                };
                let target_path = target_dir.join(&target_rel_normalized).join(stripped);
                if is_ignored(&ignore_matcher, &target_path) {
                    continue;
                }
                check_and_collect_collision(&mut root, &mut collisions, &target_path, &pattern_str);
            }
        } else {
            let source_path = source_dir.join(&pattern_normalized);
            if !source_path.path_exists() {
                continue;
            }

            if source_path.path_is_dir() {
                for entry in walk_files(&source_path) {
                    let file_source = entry.path().to_path_buf();
                    let rel_to_pattern = file_source.safe_strip_prefix(&source_path);
                    let target_path = target_dir.join(&target_rel_normalized).join(rel_to_pattern);
                    if is_ignored(&ignore_matcher, &target_path) {
                        continue;
                    }
                    check_and_collect_collision(
                        &mut root,
                        &mut collisions,
                        &target_path,
                        &pattern_str,
                    );
                }
            } else {
                let target_path = target_dir.join(&target_rel_normalized);
                if is_ignored(&ignore_matcher, &target_path) {
                    continue;
                }
                check_and_collect_collision(&mut root, &mut collisions, &target_path, &pattern_str);
            }
        }
    }

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

fn toml_quote(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{}\"", escaped)
}

fn annotate_portal_key(content: &mut String, key: &str, annotation: &str) -> bool {
    let quoted = toml_quote(key);
    let mut in_portal = false;
    let mut found = false;
    let mut insert_at = 0;

    let bytes = content.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let line_start = i;
        while i < bytes.len() && bytes[i] != b'\n' {
            i += 1;
        }
        let line = &content[line_start..i];
        if i < bytes.len() {
            i += 1;
        }

        if !in_portal {
            if line.trim() == "[portal]" {
                in_portal = true;
            }
        } else if line.trim().starts_with('[') && line.trim() != "[portal]" {
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
                    insert_at = line_start;
                    break;
                }
            }
        }
    }

    if found {
        content.insert_str(insert_at, &format!("{}\n", annotation));
        true
    } else {
        false
    }
}

fn apply_config_changes(
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

    if !new_content.contains("[portal]") {
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

fn prepare_config(
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

fn launch_editor(file_path: &Path, config_override: Option<PathBuf>) -> Result<()> {
    if cfg!(test) {
        return Ok(());
    }

    let file = file_path.to_string_lossy().into_owned();
    let (row, col) = if let Ok(content) = fs::read_to_string(&file) {
        find_portal_cursor(&content)
    } else {
        (1, 1)
    };

    let (cmd, args) = match GlobalConfig::read(config_override)? {
        GlobalConfig {
            editor_command: Some(config),
            ..
        } => {
            let params = [
                ("file", file.as_str()),
                ("row", &row.to_string()),
                ("col", &col.to_string()),
            ];
            let args = expand_args(&config.args, &params)
                .wrap_err("Failed to expand editor command parameters")?;
            (config.command, args)
        }
        _ => match env::var_os("VISUAL").or(env::var_os("EDITOR")) {
            Some(cmd) => (cmd.to_string_lossy().into_owned(), vec![file]),
            None => {
                let config_path = crate::path::global_config_path()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| "$XDG_CONFIG_HOME/dotrift/config.toml".into());
                return Err(eyre!("No editor found").with_suggestion(|| {
                    format!("Set $VISUAL or $EDITOR, or configure editor-command in {config_path}")
                }));
            }
        },
    };

    let status = Command::new(&cmd).args(&args).status();
    if let Err(ref e) = status {
        return if e.kind() == ErrorKind::NotFound {
            Err(eyre!("Editor command not found: `{cmd}`"))
        } else {
            status.map(|_| ()).wrap_err_with(|| {
                format!(
                    "Failed to launch editor: `{}`",
                    [vec![cmd], args].concat().join(" ")
                )
            })
        };
    }

    Ok(())
}

fn remove_obstructions(path: &Path, verbose: bool) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.is_dir() {
            crate::remove_dir_err!(fs::remove_dir_all(path), path)?;
            if verbose {
                output::print_removed(path);
            }
        } else {
            crate::remove_file_err!(fs::remove_file(path), path)?;
            if verbose {
                output::print_removed(path);
            }
        }
    }
    for dir in path.ancestors().skip(1) {
        let Ok(meta) = fs::symlink_metadata(dir) else {
            continue;
        };
        if meta.is_dir() {
            break;
        }
        crate::remove_file_err!(fs::remove_file(dir), dir)?;
        if verbose {
            output::print_removed(dir);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, os::unix::fs as unix_fs, path::Path};

    use crate::{command::util::tests::setup_test, config::DeployType, db::DbEntry};

    use super::*;
    use test_case::test_case;

    const FLAGS: AddFlags = AddFlags {
        copy: false,
        force: false,
        editor: None,
        no_modify: false,
    };

    fn mock_add(source_dir: &Path, path: &Path, destination: &Path, flags: AddFlags) {
        let temp_db = tempfile::tempdir().unwrap();
        run(
            GlobalFlags::new(Some(source_dir.to_path_buf()), None, None),
            path.to_path_buf(),
            Some(destination.to_path_buf()),
            flags,
            &temp_db.path().join("db"),
        )
        .unwrap();
    }

    fn mock_add_reimport(source_dir: &Path, path: &Path, flags: AddFlags, db_path: &Path) {
        run(
            GlobalFlags::new(Some(source_dir.to_path_buf()), None, None),
            path.to_path_buf(),
            None,
            flags,
            db_path,
        )
        .unwrap();
    }

    #[test_case(
        |t, _| {
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), "file".into())
        },
        |s, t| {
            assert!(!t.join("file").exists());
            assert!(s.join("file").exists());
        }; "move_relative_path"
    )]
    #[test_case(
        |t, s| {
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), s.join("file"))
        },
        |s, t| {
            assert!(!t.join("file").exists());
            assert!(s.join("file").exists());
        }; "move_absolute_path"
    )]
    #[test_case(
        |t, _| {
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), "./dir/../file".into())
        },
        |s, _| {
            assert!(s.join("file").exists());
        }; "move_normalized_path"
    )]
    #[test_case(
        |t, s| {
            fs::write(t.join("real"), "data").unwrap();
            unix_fs::symlink(t.join("real"), t.join("link")).unwrap();
            (t.join("link"), s.join("link"))
        },
        |s, t| {
            assert!(s.join("link").is_symlink());
            assert_eq!(
                fs::read_link(s.join("link")).unwrap(),
                t.join("real")
            );
        }; "move_symlink"
    )]
    #[test_case(
        |t, s| {
            fs::create_dir_all(t.join("dir/sub")).unwrap();
            fs::write(t.join("dir/sub/file"), "data").unwrap();
            (t.join("dir"), s.join("dir"))
        },
        |s, _| {
            assert!(s.join("dir").is_dir());
            assert!(s.join("dir/sub/file").exists());
        }; "move_directory"
    )]
    #[test_case(
        |t, s| {
            fs::write(t.join("file"), "data").unwrap();
            (t.join("file"), s.join("a/b/c/file"))
        },
        |s, _| {
            assert!(s.join("a/b/c/file").exists());
        }; "move_to_nested_dest"
    )]
    #[test_case(
        |_, s| {
            (s.join("nonexistent"), s.join("dest"))
        },
        |_, _| {} => panics "move"; "fail_missing_source"
    )]
    #[test_case(
        |t, s| {
            fs::write(t.join("file"), "").unwrap();
            fs::write(s.join("file"), "").unwrap();
            (t.join("file"), s.join("file"))
        },
        |_, _| {} => panics "already exists"; "fail_dest_exists"
    )]
    #[test_case(
        |t, _| {
            (t.join("file"), "../file".into())
        },
        |_, _| {} => panics "inside"; "fail_escape_source"
    )]
    fn test_add_move(
        setup: impl FnOnce(&Path, &Path) -> (PathBuf, PathBuf),
        assertion: impl FnOnce(&Path, &Path),
    ) {
        let (temp_dir, source_dir, _) = setup_test("", "", "", false);
        let (f, d) = setup(temp_dir.path(), &source_dir);

        mock_add(&source_dir, &f, &d, FLAGS);

        assertion(&source_dir, temp_dir.path());
    }

    #[test]
    fn test_add_copy() {
        let (temp_dir, source_dir, _) = setup_test("", "", "", false);

        let f = temp_dir.path().join("file");
        let d = source_dir.join("file");
        fs::write(&f, "").unwrap();
        mock_add(
            &source_dir,
            &f,
            &d,
            AddFlags {
                copy: true,
                ..FLAGS
            },
        );
        assert!(f.exists());
        assert!(d.exists());
    }

    #[test]
    fn test_add_force() {
        let (temp_dir, source_dir, _) = setup_test("", "", "", false);

        let f = temp_dir.path().join("file");
        let d = source_dir.join("file");
        fs::write(&f, "a").unwrap();
        fs::write(&d, "b").unwrap();
        mock_add(
            &source_dir,
            &f,
            &d,
            AddFlags {
                force: true,
                ..FLAGS
            },
        );
        assert_eq!(fs::read_to_string(d).unwrap(), "a");
    }

    #[test_case(
        |t, s| {
            fs::write(t.join("file"), "data").unwrap();
            (t.join("file"), vec![DbEntry {
                target_path: t.join("file"),
                deploy_type: DeployType::Copy,
                source_path: s.join("file"),
                hash: None,
                symlink_target: None,
                mtime: None,
            }])
        },
        |s| {
            assert_eq!(fs::read_to_string(s.join("file")).unwrap(), "data");
        }; "reimport_file"
    )]
    #[test_case(
        |t, s| {
            fs::write(t.join("real"), "data").unwrap();
            unix_fs::symlink(t.join("real"), t.join("link")).unwrap();
            (t.join("link"), vec![DbEntry {
                target_path: t.join("link"),
                deploy_type: DeployType::Copy,
                source_path: s.join("link"),
                hash: None,
                symlink_target: Some(t.join("real")),
                mtime: None,
            }])
        },
        |s| {
            assert!(s.join("link").is_symlink());
        }; "reimport_symlink"
    )]
    #[test_case(
        |t, s| {
            fs::create_dir_all(t.join("dir/sub")).unwrap();
            fs::write(t.join("dir/file1"), "a").unwrap();
            fs::write(t.join("dir/sub/file2"), "b").unwrap();
            (t.join("dir"), vec![
                DbEntry {
                    target_path: t.join("dir/file1"),
                    deploy_type: DeployType::Copy,
                    source_path: s.join("dest/file1"),
                    hash: None,
                    symlink_target: None,
                    mtime: None,
                },
                DbEntry {
                    target_path: t.join("dir/sub/file2"),
                    deploy_type: DeployType::Copy,
                    source_path: s.join("dest/sub/file2"),
                    hash: None,
                    symlink_target: None,
                    mtime: None,
                },
            ])
        },
        |s| {
            assert_eq!(fs::read_to_string(s.join("dest/file1")).unwrap(), "a");
            assert_eq!(fs::read_to_string(s.join("dest/sub/file2")).unwrap(), "b");
        }; "reimport_directory"
    )]
    #[test_case(
        |t, s| {
            fs::create_dir_all(t.join("dir")).unwrap();
            fs::write(t.join("dir/file1"), "").unwrap();
            fs::write(t.join("dir/file2"), "").unwrap();
            (t.join("dir"), vec![
                DbEntry {
                    target_path: t.join("dir/file1"),
                    deploy_type: DeployType::Copy,
                    source_path: s.join("file"),
                    hash: None,
                    symlink_target: None,
                    mtime: None,
                },
            ])
        },
        |s| {
            assert!(!s.join("dir").exists());
            assert!(s.join("file").exists());
        }; "reimport_directory_partial"
    )]
    fn test_add_reimport(
        setup: impl FnOnce(&Path, &Path) -> (PathBuf, Vec<DbEntry>),
        assertion: impl FnOnce(&Path),
    ) {
        let (temp_dir, source_dir, _) = setup_test("", "", "", false);
        let db_path = &temp_dir.path().join("db");
        let (p, es) = setup(temp_dir.path(), &source_dir);

        let db = Db::init(db_path).unwrap();
        for e in es {
            db.insert_or_update(&e).unwrap();
        }
        mock_add_reimport(&source_dir, &p, FLAGS, db_path);
        assertion(&source_dir);
    }

    #[test_case(
        |t| {
            fs::write(t.join("x"), "data").unwrap();
            t.join("x")
        },
        |t| {
            assert!(!t.join("x").exists());
        }
        ; "obstruction_file_at_dest"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("x")).unwrap();
            t.join("x")
        },
        |t| {
            assert!(!t.join("x").exists());
        }
        ; "obstruction_empty_dir_at_dest"
    )]
    #[test_case(
        |t| {
            let x = t.join("x");
            fs::create_dir_all(x.join("sub/nested")).unwrap();
            fs::write(x.join("sub/nested/f"), "data").unwrap();
            x
        },
        |t| {
            assert!(!t.join("x").exists());
        }
        ; "obstruction_nonempty_dir_at_dest"
    )]
    #[test_case(
        |t| {
            unix_fs::symlink(Path::new("/nonexistent"), t.join("link")).unwrap();
            t.join("link")
        },
        |t| {
            assert!(!t.join("link").is_symlink());
        }
        ; "obstruction_dangling_symlink_at_dest"
    )]
    #[test_case(
        |t| {
            let real = t.join("real");
            fs::write(&real, "data").unwrap();
            unix_fs::symlink(&real, t.join("link")).unwrap();
            t.join("link")
        },
        |t| {
            assert!(!t.join("link").is_symlink());
            assert!(t.join("real").exists());
        }
        ; "obstruction_valid_symlink_at_dest"
    )]
    #[test_case(
        |t| {
            let real = t.join("dir");
            fs::create_dir(&real).unwrap();
            fs::write(real.join("f"), "data").unwrap();
            unix_fs::symlink(&real, t.join("link")).unwrap();
            t.join("link")
        },
        |t| {
            assert!(!t.join("link").is_symlink());
            assert!(t.join("dir/f").exists());
        }
        ; "obstruction_symlink_to_dir_at_dest"
    )]
    #[test_case(
        |t| {
            t.join("ghost")
        },
        |_| {}
        ; "obstruction_nonexistent_dest"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("a"), "block").unwrap();
            t.join("a/b/c")
        },
        |t| {
            assert!(!t.join("a").exists());
        }
        ; "obstruction_file_blocking_parent"
    )]
    #[test_case(
        |t| {
            unix_fs::symlink(Path::new("/nonexistent"), t.join("a")).unwrap();
            t.join("a/b/c")
        },
        |t| {
            assert!(!t.join("a").is_symlink());
        }
        ; "obstruction_dangling_symlink_blocking_parent"
    )]
    #[test_case(
        |t| {
            let real = t.join("real");
            fs::write(&real, "data").unwrap();
            unix_fs::symlink(&real, t.join("a")).unwrap();
            t.join("a/b/c")
        },
        |t| {
            assert!(!t.join("a").is_symlink());
            assert!(t.join("real").exists());
        }
        ; "obstruction_valid_symlink_blocking_parent"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("dir")).unwrap();
            t.join("dir/target")
        },
        |t| {
            assert!(t.join("dir").is_dir());
        }
        ; "obstruction_none_with_parent"
    )]
    #[test_case(
        |t| {
            t.join("a/b/c")
        },
        |_| {}
        ; "obstruction_none_without_parent"
    )]
    fn test_remove_obstructions(setup: impl FnOnce(&Path) -> PathBuf, assert: impl FnOnce(&Path)) {
        let t = tempfile::tempdir().unwrap();
        let target = setup(t.path());
        remove_obstructions(&target, false).unwrap();
        assert(t.path());
    }

    thread_local! {
        pub static OPEN_EDITOR: RefCell<bool> = const { RefCell::new(false) };
    }

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
        "[portal]

# suffix
" => Some(r#"[portal]
"key" = "key"

# suffix
"#.to_string())
        ; "missing_key_inserted_before_trailing_content"
    )]
    fn test_apply_config_changes(analysis: PortalAnalysis, content: &str) -> Option<String> {
        apply_config_changes(content, &analysis, Path::new("/target"))
    }

    #[test_case(
        r#""" = """#,
        |t, _| {
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), "file".into())
        },
        |_, _| {
            OPEN_EDITOR.with_borrow(|b| assert!(!b));
        }        ; "editor_closed_when_destination_matches_portal"
    )]
    #[test_case(
        r#""other" = """#,
        |t, _| {
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), "file".into())
        },
        |_, _| {
            OPEN_EDITOR.with_borrow(|b| assert!(b));
        }        ; "editor_opens_when_portal_mismatch"
    )]
    #[test_case(
        "",
        |t, _| {
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), "file".into())
        },
        |_, _| {
            OPEN_EDITOR.with_borrow(|b| assert!(b));
        }        ; "editor_opens_when_portal_empty"
    )]
    #[test_case(
        r#""" = ""
"other" = "file"
"#,
        |t, s| {
            fs::write(s.join("other"), "").unwrap();
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), "file".into())
        },
        |_, _| {
            OPEN_EDITOR.with_borrow(|b| assert!(b));
        }        ; "editor_opens_when_collision_exists"
    )]
    #[test_case(
        r#""other" = "file""#,
        |t, s| {
            fs::write(s.join("other"), "").unwrap();
            fs::write(t.join("file"), "").unwrap();
            (t.join("file"), "file".into())
        },
        |_, _| {
            OPEN_EDITOR.with_borrow(|b| assert!(b));
        }        ; "editor_opens_when_collision_and_missing"
    )]
    fn test_add_open_editor(
        portal: &str,
        setup: impl FnOnce(&Path, &Path) -> (PathBuf, PathBuf),
        assertion: impl FnOnce(&Path, &Path),
    ) {
        let (temp_dir, source_dir, _) = setup_test(portal, "", "", false);
        let (f, d) = setup(temp_dir.path(), &source_dir);

        mock_add(&source_dir, &f, &d, FLAGS);

        assertion(&source_dir, temp_dir.path());
    }
}
