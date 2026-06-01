use std::{
    collections::HashMap,
    fs::{self, symlink_metadata},
    io::{BufWriter, Write},
    os::unix::fs::{self as unix_fs, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

use glob::Pattern;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use memmap2::Mmap;
use miette::{Context, Report, Result, miette};

use crate::{
    cli::{ApplyFlags, GlobalFlags},
    command::{
        prompt::{CollisionOptions, prompt_collision},
        tree::{Node, build_tree},
        util::{
            GLOB_OPTION, PathExt, StripPrefixOrSelf, clean_up, clone_file, hash_file, is_managed,
            read_mtime, resolve_portal_entries, resolve_target,
        },
    },
    config::{Config, DeployType, FileMode, Rules},
    create_file_err,
    db::{Db, DbEntry},
    global_config::GlobalConfig,
    mmap_template_err, open_template_err, output, parse_template_err, render_template_err,
    templater::{data, function::BuiltinFunctions},
    write_file_err,
};

#[derive(Default, Debug, PartialEq)]
pub struct PortalEntry {
    pub source: PathBuf,
    pub deploy_type: DeployType,
    pub mode: Option<FileMode>,
}

struct TemplateContext {
    variables: HashMap<String, templater::value::Value>,
    functions: BuiltinFunctions,
}

impl TemplateContext {
    fn build(source_dir: &Path, db: &Db) -> Result<Self> {
        let data = data::TemplateData::read(source_dir)?;
        Ok(Self {
            variables: data.resolve_variables(db)?,
            functions: BuiltinFunctions::new(),
        })
    }
}

pub fn run(global_flags: GlobalFlags, db_path: &Path, flags: ApplyFlags) -> Result<()> {
    let source_dir = global_flags.source()?;
    let target_override = global_flags.target()?;
    let config_override = global_flags.config()?;
    let verbose = global_flags.verbose;

    let db = Db::init(db_path)?;
    let template_ctx = TemplateContext::build(&source_dir, &db)?;
    let config = Config::read_templated(
        &source_dir,
        &template_ctx.variables,
        &template_ctx.functions,
    )?;

    let target_dir = resolve_target(&source_dir, target_override, &config)?;

    let ignore_matcher = build_ignore(&config.ignore, &target_dir)?;

    let mut portal_entries =
        resolve_portals(&source_dir, &target_dir, &config.portal, &ignore_matcher)?;

    apply_rules(&target_dir, &mut portal_entries, &config.rule)?;

    let remove_count = if flags.clean_up {
        clean_up(
            &portal_entries,
            &db,
            flags.dry_run,
            flags.prune_empty_dirs,
            verbose,
            false,
        )?
    } else {
        0
    };

    let tree = build_tree(portal_entries).wrap_err("check portal entries for conflicting target paths, a file and a directory cannot share the same target path")?;

    if flags.dry_run {
        let create_count = print_tree(Path::new("/"), &tree)?;
        let mut parts = Vec::with_capacity(2);
        if create_count > 0 {
            parts.push(if create_count == 1 {
                "1 create".to_string()
            } else {
                format!("{} creates", create_count)
            });
        }
        if remove_count > 0 {
            parts.push(if remove_count == 1 {
                "1 removal".to_string()
            } else {
                format!("{} removals", remove_count)
            });
        }
        if !parts.is_empty() {
            output::print_summary(parts.join(", "));
        }
        return Ok(());
    }

    let overwrite_identical = GlobalConfig::read(config_override)?.overwrite_identical;
    traverse_tree(
        Path::new("/"),
        &tree,
        &db,
        overwrite_identical,
        verbose,
        &template_ctx,
    )?;

    Ok(())
}

pub fn build_ignore(patterns: &[String], target_dir: &Path) -> Result<Gitignore> {
    let mut builder = GitignoreBuilder::new(target_dir);

    let _ = builder.add_line(None, "dotrift.toml");
    let _ = builder.add_line(None, "dotrift_data.toml");

    for pattern in patterns {
        builder
            .add_line(None, pattern)
            .map_err(|e| miette!(e))
            .wrap_err_with(|| format!("invalid ignore pattern: `{pattern}`"))
            .map_err(|e| {
                miette!(
                    help = "use gitignore-style patterns, see gitignore documentation for syntax",
                    "{e}"
                )
            })?;
    }
    builder
        .build()
        .map_err(|e| miette!(e))
        .wrap_err("failed to compile ignore patterns")
}

pub fn resolve_portals(
    source_dir: &Path,
    target_dir: &Path,
    portals: &HashMap<String, PathBuf>,
    ignore_matcher: &Gitignore,
) -> Result<HashMap<PathBuf, PortalEntry>> {
    let mut portal_entries: HashMap<PathBuf, PortalEntry> = HashMap::new();

    resolve_portal_entries(
        source_dir,
        target_dir,
        portals,
        ignore_matcher,
        false,
        |source_path, target_path, _| {
            insert_portal_entry(&mut portal_entries, target_path, source_path)
        },
    )?;

    Ok(portal_entries)
}

fn insert_portal_entry(
    portal_entries: &mut HashMap<PathBuf, PortalEntry>,
    target_path: PathBuf,
    source_path: PathBuf,
) -> Result<()> {
    if let Some(existing) = portal_entries.insert(
        target_path.clone(),
        PortalEntry {
            source: source_path.clone(),
            deploy_type: DeployType::default(),
            mode: None,
        },
    ) {
        return Err(miette!(
            "target path collision at `{}`: source 1: `{}`, source 2: `{}`",
            target_path.display(),
            existing.source.display(),
            source_path.display()
        ));
    }
    Ok(())
}

fn apply_rules(
    target_dir: &Path,
    portal_entries: &mut HashMap<PathBuf, PortalEntry>,
    rules: &Rules,
) -> Result<()> {
    let mut compiled = Vec::with_capacity(rules.len());
    for (p, r) in rules {
        compiled.push((crate::glob_err!(Pattern::new(p), p)?, r));
    }

    for (path, portal_entry) in portal_entries.iter_mut() {
        let rel = path.safe_strip_prefix(target_dir);
        for (pattern, rule) in &compiled {
            if !pattern.matches_path_with(rel, GLOB_OPTION) {
                continue;
            }
            if let Some(t) = &rule.deploy_type {
                portal_entry.deploy_type = *t;
            }
            if let Some(m) = &rule.mode {
                portal_entry.mode = Some(*m);
            }
        }
    }
    Ok(())
}

fn print_tree(path: &Path, node: &Node) -> Result<usize> {
    let mut count = 0;
    match node {
        Node::Dir(children) => {
            if path != Path::new("/") {
                output::print_dry_create_dir(path);
            }
            for (name, child) in children {
                count += print_tree(&path.join(name), child)?;
            }
        }
        Node::File(entry) => {
            count += 1;
            output::print_dry_create_file(path, &entry.source, entry.deploy_type);
        }
        Node::Claim(_) => {
            #[cfg(test)]
            unreachable!()
        }
    }

    Ok(count)
}

fn traverse_tree(
    target: &Path,
    node: &Node,
    db: &Db,
    overwrite_identical: bool,
    verbose: bool,
    template_ctx: &TemplateContext,
) -> Result<()> {
    match node {
        Node::Dir(children) => {
            if deploy_dir(target, db, verbose)? {
                return Ok(());
            }
            for (name, child) in children {
                traverse_tree(
                    &target.join(name),
                    child,
                    db,
                    overwrite_identical,
                    verbose,
                    template_ctx,
                )?;
            }
        }
        Node::File(entry) => {
            deploy_file(
                target,
                entry,
                db,
                overwrite_identical,
                verbose,
                template_ctx,
            )?;
        }
        Node::Claim(_) => {
            #[cfg(test)]
            unreachable!()
        }
    }
    Ok(())
}

fn abort_deploy(at: &Path) -> Report {
    miette!("aborted at `{}`", at.display())
}

fn deploy_dir(path: &Path, db: &Db, verbose: bool) -> Result<bool> {
    if path.path_exists() {
        if path.path_is_dir() {
            return Ok(false);
        }
        let choice = prompt_collision(None, path, true, false);
        match choice {
            CollisionOptions::Skip => return Ok(true),
            CollisionOptions::Overwrite => {
                crate::remove_file_err!(fs::remove_file(path), path)?;
                if verbose {
                    output::print_removed(path);
                }
                db.delete_entry(path)?;
            }
            CollisionOptions::Quit => {
                return Err(abort_deploy(path));
            }
            CollisionOptions::Diff => unreachable!(),
        }
    }
    crate::create_dir_err!(fs::create_dir_all(path), path)?;
    if verbose {
        output::print_created_dir(path);
    }
    Ok(false)
}

fn is_identical(
    target: &Path,
    source: &Path,
    deploy_type: DeployType,
    source_hash: &mut Option<u64>,
    target_hash: &mut Option<u64>,
) -> bool {
    match deploy_type {
        DeployType::Symlink => {
            target.path_is_symlink() && fs::read_link(target).is_ok_and(|l| l == source)
        }
        DeployType::Copy if source.path_is_symlink() => {
            target.path_is_symlink()
                && fs::read_link(source)
                    .is_ok_and(|src_dest| fs::read_link(target).is_ok_and(|l| l == src_dest))
        }
        DeployType::Copy => {
            source.path_is_file()
                && target.path_is_file()
                && symlink_metadata(source)
                    .is_ok_and(|m1| symlink_metadata(target).is_ok_and(|m2| m1.size() == m2.size()))
                && hash_file(source).is_ok_and(|h1| {
                    *source_hash = Some(h1);
                    hash_file(target).is_ok_and(|h2| {
                        *target_hash = Some(h2);
                        h1 == h2
                    })
                })
        }
        DeployType::Tmpl => false,
    }
}

fn deploy_file(
    target: &Path,
    entry: &PortalEntry,
    db: &Db,
    overwrite_identical: bool,
    verbose: bool,
    template_ctx: &TemplateContext,
) -> Result<()> {
    let mut source_hash = None;
    let mut target_hash = None;

    if target.path_exists() {
        if target.path_is_dir() {
            let choice = prompt_collision(Some(&entry.source), target, false, true);
            match choice {
                CollisionOptions::Skip => return Ok(()),
                CollisionOptions::Overwrite => {
                    crate::remove_dir_err!(fs::remove_dir_all(target), target)?;
                    if verbose {
                        output::print_removed(target);
                    }
                    db.delete_entry_with_prefix(target)?;
                }
                CollisionOptions::Quit => {
                    return Err(abort_deploy(target));
                }
                CollisionOptions::Diff => unreachable!(),
            }
        } else {
            let identical = is_identical(
                target,
                &entry.source,
                entry.deploy_type,
                &mut source_hash,
                &mut target_hash,
            );
            if identical {
                if overwrite_identical {
                    update_db(target, entry, db, target_hash)?;
                }
                return Ok(());
            }

            #[cfg(test)]
            {
                tests::CHECK_MANAGED.set(true);
            }
            let managed = is_managed(target, db, target_hash);
            if !managed {
                let choice = prompt_collision(Some(&entry.source), target, false, false);
                match choice {
                    CollisionOptions::Skip => return Ok(()),
                    CollisionOptions::Overwrite => {}
                    CollisionOptions::Quit => {
                        return Err(abort_deploy(target));
                    }
                    CollisionOptions::Diff => unreachable!(),
                }
            }
        }
    }

    match entry.deploy_type {
        DeployType::Symlink => {
            let _ = fs::remove_file(target);
            crate::symlink_err!(
                unix_fs::symlink(&entry.source, target),
                target,
                &entry.source
            )?
        }
        DeployType::Copy => {
            clone_file(&entry.source, target)?;
        }
        DeployType::Tmpl => {
            let src = entry
                .source
                .clone()
                .canonicalize()
                .map_err(|e| miette!(e))
                .wrap_err_with(|| format!("failed to resolve `{}`", entry.source.display()))?;
            let file = open_template_err!(fs::File::open(&src), &src)?;

            // SAFETY: This process has exclusive access to the file — opened read-only,
            // no concurrent writer modifies or truncates it while mapped.
            let mmap = mmap_template_err!(unsafe { Mmap::map(&file) }, &src)?;

            let tmpl = parse_template_err!(templater::Template::from_mmap(mmap), &src)?;
            let out = create_file_err!(fs::File::create(target), target)?;
            let mut writer = BufWriter::new(out);
            render_template_err!(
                tmpl.render(
                    &mut writer,
                    &template_ctx.variables,
                    &template_ctx.functions
                ),
                &src
            )?;
            write_file_err!(writer.flush(), target)?;
        }
    }
    match entry.deploy_type {
        DeployType::Copy | DeployType::Tmpl => {
            if let Some(mode) = entry.mode
                && target.path_is_file()
            {
                fs::set_permissions(target, fs::Permissions::from_mode(mode.0 as u32))
                    .map_err(|e| miette!(e))
                    .wrap_err_with(|| {
                        format!("failed to set permissions on `{}`", target.display())
                    })?;
            }
        }
        _ => {}
    }
    if verbose {
        output::print_created_file(target, &entry.source, entry.deploy_type);
    }

    update_db(target, entry, db, source_hash)?;
    Ok(())
}

fn update_db(target: &Path, entry: &PortalEntry, db: &Db, source_hash: Option<u64>) -> Result<()> {
    let is_regular = target.path_is_file();
    db.insert_or_update(&DbEntry {
        deploy_type: entry.deploy_type,
        source_path: entry.source.clone(),
        hash: if is_regular {
            Some(match entry.deploy_type {
                DeployType::Tmpl => hash_file(target)?,
                _ => source_hash
                    .map(Ok)
                    .unwrap_or_else(|| hash_file(&entry.source))?,
            })
        } else {
            None
        },
        symlink_target: if entry.deploy_type == DeployType::Copy && entry.source.path_is_symlink() {
            Some(crate::read_link_err!(
                fs::read_link(&entry.source),
                &entry.source
            )?)
        } else {
            None
        },
        mtime: if is_regular { read_mtime(target) } else { None },
        target_path: target.to_path_buf(),
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::command::{
        prompt::tests::PROMPT_SELECTION,
        util::{assert_captured_output, tests::setup_test},
    };

    use super::*;
    use std::{cell::RefCell, fs};
    use tempfile::tempdir;
    use test_case::test_case;

    #[macro_export]
    macro_rules! portal_entries {
        ($(($s:literal, $t:literal)),*) => {
            HashMap::from_iter([$(($t.into(), PortalEntry { source: $s.into(), ..Default::default() })),*])
        };
        ($(($s:literal, $t:literal, $a:ident, $m:expr)),*) => {
            HashMap::from_iter([$(($t.into(), PortalEntry { source: $s.into(), deploy_type: DeployType::$a, mode: $m })),*])
        };
    }

    fn flatten(
        map: HashMap<PathBuf, PortalEntry>,
        temp_dir: &Path,
    ) -> HashMap<PathBuf, PortalEntry> {
        map.into_iter()
            .map(|(p, f)| {
                (
                    p.strip_prefix(temp_dir.join("target"))
                        .unwrap()
                        .to_path_buf(),
                    PortalEntry {
                        source: f
                            .source
                            .strip_prefix(temp_dir.join("source"))
                            .unwrap()
                            .to_path_buf(),
                        ..f
                    },
                )
            })
            .collect()
    }

    #[test_case("" => HashMap::new(); "literal_empty")]
    #[test_case(r#""a.txt" = "A.txt""# => portal_entries!(("a.txt", "A.txt")); "literal_file")]
    #[test_case(r#""subdir" = "dir""# => portal_entries!(("subdir/c.txt", "dir/c.txt"), ("subdir/d.txt", "dir/d.txt")); "literal_dir")]
    #[test_case(r#""" = """# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt"), ("subdir/c.txt", "subdir/c.txt"), ("subdir/d.txt", "subdir/d.txt")); "literal_root")]
    #[test_case(r#""./a.txt" = "./b.txt""# => portal_entries!(("a.txt", "b.txt")); "literal_dot_normalized")]
    #[test_case(r#""subdir/dir/../c.txt" = "dist/../root/c.txt""# => portal_entries!(("subdir/c.txt", "root/c.txt")); "literal_path_traversal")]
    #[test_case(r#""../../a.txt" = "../../a.txt""# => portal_entries!(("a.txt", "a.txt")); "literal_path_escape_clamped")]
    #[test_case(r#""*.rs" = """# => portal_entries!(); "glob_no_match")]
    #[test_case(r#""*.txt" = "root""# => portal_entries!(("a.txt", "root/a.txt"), ("b.txt", "root/b.txt")); "glob_shallow_pattern")]
    #[test_case(r#""**/*" = ".""# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt"), ("subdir/c.txt", "subdir/c.txt"), ("subdir/d.txt", "subdir/d.txt")); "glob_recursive_all")]
    #[test_case(r#""**/*.txt" = "files""# => portal_entries!(("a.txt", "files/a.txt"), ("b.txt", "files/b.txt"), ("subdir/c.txt", "files/subdir/c.txt"), ("subdir/d.txt", "files/subdir/d.txt")); "glob_recursive_pattern")]
    #[test_case(r#""**/c.txt" = "out""# => portal_entries!(("subdir/c.txt", "out/subdir/c.txt")); "glob_recursive_prefix")]
    #[test_case(r#""subdir/**/*.txt" = "out""# => portal_entries!(("subdir/c.txt", "out/c.txt"), ("subdir/d.txt", "out/d.txt")); "glob_recursive_middle")]
    #[test_case(r#""a.txt" = "A.txt"
"a.*" = """# => portal_entries!(("a.txt", "a.txt"), ("a.txt", "A.txt")); "multiple_same_source")]
    #[test_case(r#""a.txt" = "same"
"b.txt" = "same""# => panics "collision"; "multiple_same_target")]
    #[test_case(r#""**/*" = "."
"a.txt" = "a.txt""# => panics "collision"; "literal_glob_same_target")]
    #[test_case(r#""foo" = "bar""# => panics "does not exist"; "non_existing")]
    fn test_resolve_portals(portal: &str) -> HashMap<PathBuf, PortalEntry> {
        let (temp_dir, source_dir, target_dir) = setup_test(portal, "", "", true);
        let config = Config::read(&source_dir).unwrap();
        let ignore_matcher = build_ignore(&config.ignore, &target_dir).unwrap();
        let portal_entries =
            resolve_portals(&source_dir, &target_dir, &config.portal, &ignore_matcher).unwrap();
        flatten(portal_entries, temp_dir.path())
    }

    #[test_case("" => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt"), ("subdir/c.txt", "subdir/c.txt"), ("subdir/d.txt", "subdir/d.txt")); "empty")]
    #[test_case(r#""*.txt""# => portal_entries!(); "glob_all_files")]
    #[test_case(r#""**""# => portal_entries!(); "glob_everything")]
    #[test_case(r#""/*.txt""# => portal_entries!(("subdir/c.txt", "subdir/c.txt"), ("subdir/d.txt", "subdir/d.txt")); "glob_anchored")]
    #[test_case(r#""subdir/*""# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt")); "glob_dir_contents")]
    #[test_case(r#""**/c.txt""# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt"), ("subdir/d.txt", "subdir/d.txt")); "glob_file_anywhere")]
    #[test_case(r#""subdir/**""# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt")); "glob_double_star_dir")]
    #[test_case(r#""subdir/""# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt")); "dir_trailing_slash")]
    #[test_case(r#""*.txt", "!dotrift.toml""# => portal_entries!(("dotrift.toml", "dotrift.toml")); "negate_selective")]
    #[test_case(r#""!a.txt""# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt"), ("subdir/c.txt", "subdir/c.txt"), ("subdir/d.txt", "subdir/d.txt")); "negate_only")]
    #[test_case(r#""*.txt", "!a.txt", "!b.txt""# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt")); "negate_multiple")]
    #[test_case(r#""!nonexistent.txt""# => portal_entries!(("a.txt", "a.txt"), ("b.txt", "b.txt"), ("subdir/c.txt", "subdir/c.txt"), ("subdir/d.txt", "subdir/d.txt")); "negate_nonexistent")]
    #[test_case(r#""a.txt", "b.txt""# => portal_entries!(("subdir/c.txt", "subdir/c.txt"), ("subdir/d.txt", "subdir/d.txt")); "multiple_literal")]
    fn test_ignore(ignore: &str) -> HashMap<PathBuf, PortalEntry> {
        let (temp_dir, source_dir, target_dir) = setup_test(r#""" = """#, ignore, "", true);
        let config = Config::read(&source_dir).unwrap();
        let ignore_matcher = build_ignore(&config.ignore, &target_dir).unwrap();
        let portal_entries =
            resolve_portals(&source_dir, &target_dir, &config.portal, &ignore_matcher).unwrap();
        flatten(portal_entries, temp_dir.path())
    }

    #[test_case("" => portal_entries!(
        ("a.txt",         "a.txt",         Symlink, None),
        ("b.txt",         "b.txt",         Symlink, None),
        ("subdir/c.txt",  "subdir/c.txt",  Symlink, None),
        ("subdir/d.txt",  "subdir/d.txt",  Symlink, None)
    ); "empty")]
    #[test_case(r#""*.txt" = { type = "copy" }"# => portal_entries!(
        ("a.txt",         "a.txt",         Copy,    None),
        ("b.txt",         "b.txt",         Copy,    None),
        ("subdir/c.txt",  "subdir/c.txt",  Symlink, None),
        ("subdir/d.txt",  "subdir/d.txt",  Symlink, None)
    ); "selective_type")]
    #[test_case(r#""*.txt" = { mode = "600" }"# => portal_entries!(
        ("a.txt",         "a.txt",         Symlink, Some(FileMode(0o600))),
        ("b.txt",         "b.txt",         Symlink, Some(FileMode(0o600))),
        ("subdir/c.txt",  "subdir/c.txt",  Symlink, None),
        ("subdir/d.txt",  "subdir/d.txt",  Symlink, None)
    ); "rule_mode")]
    #[test_case(r#""*.txt" = { mode = "600" }
    "a.txt" = { type = "copy" }"# => portal_entries!(
        ("a.txt",         "a.txt",         Copy,    Some(FileMode(0o600))),
        ("b.txt",         "b.txt",         Symlink, Some(FileMode(0o600))),
        ("subdir/c.txt",  "subdir/c.txt",  Symlink, None),
        ("subdir/d.txt",  "subdir/d.txt",  Symlink, None)
    ); "rule_merge")]
    #[test_case(r#""*.txt" = { type = "symlink", mode = "600" }
    "a.txt" = { type = "copy", mode = "700" }"# => portal_entries!(
        ("a.txt",         "a.txt",         Copy,    Some(FileMode(0o700))),
        ("b.txt",         "b.txt",         Symlink, Some(FileMode(0o600))),
        ("subdir/c.txt",  "subdir/c.txt",  Symlink, None),
        ("subdir/d.txt",  "subdir/d.txt",  Symlink, None)
    ); "rule_override")]
    #[test_case(r#""subdir/*.txt" = { mode = "600" }"# => portal_entries!(
        ("a.txt",         "a.txt",         Symlink, None),
        ("b.txt",         "b.txt",         Symlink, None),
        ("subdir/c.txt",  "subdir/c.txt",  Symlink, Some(FileMode(0o600))),
        ("subdir/d.txt",  "subdir/d.txt",  Symlink, Some(FileMode(0o600)))
    ); "subdir_rule")]
    #[test_case(r#""**/*.txt" = { type = "copy" }"# => portal_entries!(
        ("a.txt",         "a.txt",         Copy, None),
        ("b.txt",         "b.txt",         Copy, None),
        ("subdir/c.txt",  "subdir/c.txt",  Copy, None),
        ("subdir/d.txt",  "subdir/d.txt",  Copy, None)
    ); "recursive_glob")]
    #[test_case(r#""*.rs" = { type = "copy" }"# => portal_entries!(
        ("a.txt",         "a.txt",         Symlink, None),
        ("b.txt",         "b.txt",         Symlink, None),
        ("subdir/c.txt",  "subdir/c.txt",  Symlink, None),
        ("subdir/d.txt",  "subdir/d.txt",  Symlink, None)
    ); "no_match")]
    fn test_apply_rules(rule: &str) -> HashMap<PathBuf, PortalEntry> {
        let (temp_dir, source_dir, target_dir) = setup_test(r#""" = """#, "", rule, true);
        let config = Config::read(&source_dir).unwrap();
        let ignore_matcher = build_ignore(&config.ignore, &target_dir).unwrap();
        let mut portal_entries =
            resolve_portals(&source_dir, &target_dir, &config.portal, &ignore_matcher).unwrap();
        apply_rules(&target_dir, &mut portal_entries, &config.rule).unwrap();
        flatten(portal_entries, temp_dir.path())
    }

    #[test_case(
        |s, t| {
            unix_fs::symlink(s.join("src"), t.join("target")).unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Symlink)
        => true; "symlink_identical")]
    #[test_case(
        |s, t| {
            unix_fs::symlink(s.join("other"), t.join("target")).unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Symlink)
        => false; "symlink_not_identical")]
    #[test_case(
        |s, _| {
            fs::write(s.join("src"), "").unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Symlink)
        => false; "symlink_target_not_symlink")]
    #[test_case(
        |s, t| {
            fs::write(s.join("src"), "a").unwrap();
            fs::write(t.join("target"), "a").unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => true; "copy_file_identical")]
    #[test_case(
        |s, t| {
            fs::write(s.join("src"), "a").unwrap();
            fs::write(t.join("target"), "b").unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => false; "copy_file_not_identical")]
    #[test_case(
        |s, t| {
            fs::write(s.join("src"), "a").unwrap();
            unix_fs::symlink(s.join("src"), t.join("target")).unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => false; "copy_file_target_is_symlink")]
    #[test_case(
        |s, _| {
            unix_fs::symlink(Path::new("/a"), s.join("src")).unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => false; "copy_symlink_source_target_missing")]
    #[test_case(
        |s, t| {
            unix_fs::symlink(Path::new("/a"), s.join("src")).unwrap();
            unix_fs::symlink(Path::new("/a"), t.join("target")).unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => true; "copy_symlink_source_identical")]
    #[test_case(
        |s, t| {
            unix_fs::symlink(Path::new("/a"), s.join("src")).unwrap();
            unix_fs::symlink(Path::new("/b"), t.join("target")).unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => false; "copy_symlink_source_not_identical")]
    #[test_case(
        |s, t| {
            unix_fs::symlink(Path::new("/a"), s.join("src")).unwrap();
            fs::write(t.join("target"), "").unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Copy)
        => false; "copy_symlink_source_target_is_file")]
    #[test_case(
        |s, t| {
            fs::write(s.join("src"), "same").unwrap();
            fs::write(t.join("target"), "same").unwrap();
        },
        |s, t| (t.join("target"), s.join("src"), DeployType::Tmpl)
        => false; "tmpl_always_false"
    )]
    fn test_is_identical(
        setup: impl FnOnce(&Path, &Path),
        paths: impl FnOnce(&Path, &Path) -> (PathBuf, PathBuf, DeployType),
    ) -> bool {
        let temp_dir = tempdir().unwrap();
        let source_dir = temp_dir.path().join("source");
        let target_dir = temp_dir.path().join("target");
        fs::create_dir_all(&source_dir).unwrap();
        fs::create_dir_all(&target_dir).unwrap();

        setup(&source_dir, &target_dir);
        let (target, source, deploy_type) = paths(&source_dir, &target_dir);
        is_identical(&target, &source, deploy_type, &mut None, &mut None)
    }

    const FLAGS: ApplyFlags = ApplyFlags {
        dry_run: false,
        clean_up: false,
        prune_empty_dirs: false,
    };

    fn mock_apply(
        source_dir: &Path,
        target_dir: &Path,
        db_path: &Path,
        flags: ApplyFlags,
    ) -> Result<()> {
        run(
            GlobalFlags::new(
                Some(source_dir.to_path_buf()),
                Some(target_dir.to_path_buf()),
                None,
            ),
            db_path,
            flags,
        )
    }

    thread_local! {
        pub static CHECK_MANAGED: RefCell<bool> = const { RefCell::new(false) };
    }

    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "").unwrap();
        },
        |s, t| {
            assert!(t.join("file").exists());
            assert!(t.join("file").is_symlink());
            assert_eq!(fs::read_link(t.join("file")).unwrap(), s.join("file"));
        },
        DeployType::Symlink; "symlink_fresh"
    )]
    #[test_case(
        |s, _| {
            fs::create_dir_all(s.join("dir")).unwrap();
            fs::write(s.join("dir/file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir/file").exists());
        },
        DeployType::Symlink; "symlink_nested"
    )]
    #[test_case(
        |s, _| {
            unix_fs::symlink(Path::new("/a"), s.join("file")).unwrap();
        },
        |s, t| {
            assert_eq!(fs::read_link(t.join("file")).unwrap(), s.join("file"));
        },
        DeployType::Symlink; "symlink_broken_source"
    )]
    #[test_case(
        |s, _| {
            unix_fs::symlink(Path::new("/a"), s.join("file")).unwrap();
        },
        |_, t| {
            assert_eq!(fs::read_link(t.join("file")).unwrap(), Path::new("/a"));
        },
        DeployType::Copy; "copy_symlink_source_fresh"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("file").exists());
            assert!(!t.join("file").is_symlink());
        },
        DeployType::Copy; "copy_fresh"
    )]
    #[test_case(
        |s, _| {
            fs::create_dir_all(s.join("dir/sub")).unwrap();
            fs::write(s.join("dir/file1"), "a").unwrap();
            fs::write(s.join("dir/sub/file2"), "b").unwrap();
        },
        |_, t| {
            assert_eq!(fs::read_to_string(t.join("dir/file1")).unwrap(), "a");
            assert_eq!(fs::read_to_string(t.join("dir/sub/file2")).unwrap(), "b");
        },
        DeployType::Copy; "copy_nested_dir_fresh"
    )]
    #[test_case(
        |s, t| {
            unix_fs::symlink(Path::new("/a"), s.join("file")).unwrap();
            unix_fs::symlink(s.join("file"), t.join("file")).unwrap();
        },
        |_, _| {
            assert!(!CHECK_MANAGED.with_borrow(|b| *b));
        },
        DeployType::Symlink; "symlink_identical"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("file"), "a").unwrap();
            fs::write(t.join("file"), "a").unwrap();
        },
        |_, _| {
            assert!(!CHECK_MANAGED.with_borrow(|b| *b));
        },
        DeployType::Copy; "copy_identical_file"
    )]
    #[test_case(
        |s, t| {
            unix_fs::symlink(Path::new("/a"), s.join("file")).unwrap();
            unix_fs::symlink(Path::new("/a"), t.join("file")).unwrap();
        },
        |_, _| {
            assert!(!CHECK_MANAGED.with_borrow(|b| *b));
        },
        DeployType::Copy; "copy_identical_symlink"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("dir"), "").unwrap();
            fs::create_dir_all(t.join("dir")).unwrap();
            fs::write(t.join("dir/file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir/file").exists());
        },
        DeployType::Symlink; "symlink_file_vs_dir_skip"
    )]
    #[test_case(
        |s, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::write(s.join("dir"), "").unwrap();
            fs::create_dir_all(t.join("dir")).unwrap();
            fs::write(t.join("dir/file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir").is_file());
        },
        DeployType::Symlink; "symlink_file_vs_dir_overwrite"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("dir"), "").unwrap();
            fs::create_dir_all(t.join("dir")).unwrap();
            fs::write(t.join("dir/file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir/file").exists());
        },
        DeployType::Copy; "copy_file_vs_dir_skip"
    )]
    #[test_case(
        |s, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::write(s.join("dir"), "").unwrap();
            fs::create_dir_all(t.join("dir")).unwrap();
            fs::write(t.join("dir/file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir").is_file());
        },
        DeployType::Copy; "copy_file_vs_dir_overwrite"
    )]
    #[test_case(
        |s, t| {
            fs::create_dir_all(s.join("dir")).unwrap();
            fs::write(s.join("dir/file"), "").unwrap();
            fs::write(t.join("dir"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir").is_file());
        },
        DeployType::Symlink; "symlink_dir_vs_file_skip"
    )]
    #[test_case(
        |s, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::create_dir_all(s.join("dir")).unwrap();
            fs::write(s.join("dir/file"), "").unwrap();
            fs::write(t.join("dir"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir/file").exists());
        },
        DeployType::Symlink; "symlink_dir_vs_file_overwrite"
    )]
    #[test_case(
        |s, t| {
            fs::create_dir_all(s.join("dir")).unwrap();
            fs::write(s.join("dir/file"), "").unwrap();
            fs::write(t.join("dir"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir").is_file());
        },
        DeployType::Copy; "copy_dir_vs_file_skip"
    )]
    #[test_case(
        |s, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::create_dir_all(s.join("dir")).unwrap();
            fs::write(s.join("dir/file"), "").unwrap();
            fs::write(t.join("dir"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("dir").exists());
            assert!(t.join("dir/file").exists());
        },
        DeployType::Copy; "copy_dir_vs_file_overwrite"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("file"), "").unwrap();
            fs::write(t.join("file"), "").unwrap();
        },
        |_, t| {
            assert!(!t.join("file").is_symlink());
        },
        DeployType::Symlink; "symlink_file_vs_file_skip"
    )]
    #[test_case(
        |s, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::write(s.join("file"), "").unwrap();
            fs::write(t.join("file"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("file").is_symlink());
        },
        DeployType::Symlink; "symlink_file_vs_file_overwrite"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("file"), "a").unwrap();
            fs::write(t.join("file"), "b").unwrap();
        },
        |_, t| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "b");
        },
        DeployType::Copy; "copy_file_vs_file_skip"
    )]
    #[test_case(
        |s, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::write(s.join("file"), "a").unwrap();
            fs::write(t.join("file"), "b").unwrap();
        },
        |_, t| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "a");
        },
        DeployType::Copy; "copy_file_vs_file_overwrite"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("file1"), "").unwrap();
            PROMPT_SELECTION.set(CollisionOptions::Quit);
            fs::write(s.join("file2"), "").unwrap();
            fs::write(t.join("file2"), "").unwrap();
        },
        |_, t| {
            assert!(t.join("file1").exists());
            assert!(!t.join("file2").exists());
        },
        DeployType::Symlink => panics "abort"; "quit_symlink"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("file1"), "").unwrap();
            PROMPT_SELECTION.set(CollisionOptions::Quit);
            fs::write(s.join("file"), "a").unwrap();
            fs::write(t.join("file"), "b").unwrap();
        },
        |_, t| {
            assert!(t.join("file1").exists());
            assert!(!t.join("file2").exists());
        },
        DeployType::Copy => panics "abort"; "quit_copy"
    )]
    #[test_case(
        |s, t| {
            fs::create_dir_all(t.join("real_dir")).unwrap();
            unix_fs::symlink(t.join("real_dir"), s.join("link_dir")).unwrap();
        },
        |s, t| {
            assert!(t.join("link_dir").is_symlink());
            assert_eq!(fs::read_link(t.join("link_dir")).unwrap(), s.join("link_dir"));
        },
        DeployType::Symlink; "symlink_to_dir_as_source"
    )]
    #[test_case(
        |s, t| {
            fs::create_dir_all(t.join("real_dir")).unwrap();
            unix_fs::symlink(t.join("real_dir"), s.join("link_dir")).unwrap();
        },
        |_, t| {
            assert!(t.join("link_dir").is_symlink());
            assert_eq!(fs::read_link(t.join("link_dir")).unwrap(), t.join("real_dir"));
        },
        DeployType::Copy; "copy_symlink_to_dir_as_source"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("dotrift_data.toml"), r#"[variable]
name = "world""#).unwrap();
            fs::write(s.join("file"), "Hello {{ name }}").unwrap();
        },
        |_, t| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "Hello world");
        },
        DeployType::Tmpl; "tmpl_fresh"
    )]
    #[test_case(
        |s, t| {
            fs::write(s.join("dotrift_data.toml"), r#"[variable]
name = "world""#).unwrap();
            fs::write(t.join("template"), "Hello {{ name }}").unwrap();
            unix_fs::symlink(t.join("template"), s.join("file")).unwrap();
        },
        |_, t| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "Hello world");
        },
        DeployType::Tmpl; "tmpl_symlink_source_fresh"
    )]
    fn test_apply(
        setup: impl FnOnce(&Path, &Path),
        assert: impl FnOnce(&Path, &Path),
        deploy_type: DeployType,
    ) {
        let (temp_dir, source_dir, target_dir) = setup_test(
            r#""" = """#,
            "",
            match deploy_type {
                DeployType::Symlink => "",
                DeployType::Copy => r#""**/*" = { type = "copy" }"#,
                DeployType::Tmpl => r#""**/*" = { type = "tmpl" }"#,
            },
            false,
        );
        setup(&source_dir, &target_dir);
        mock_apply(&source_dir, &target_dir, &temp_dir.path().join("db"), FLAGS).unwrap();
        assert(&source_dir, &target_dir);
    }

    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, _| {},
        |s, t, db| {
            assert!(t.join("file").exists());
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert_eq!(entry.hash.unwrap(), hash_file(&s.join("file")).unwrap());
        },
        DeployType::Copy; "copy_reapply_identical"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, _| {},
        |s, t, db| {
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Symlink);
            assert_eq!(fs::read_link(t.join("file")).unwrap(), s.join("file"));
            assert!(entry.hash.is_none());
        },
        DeployType::Symlink; "symlink_reapply_identical"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |s, _| {
            fs::write(s.join("file"), "b").unwrap();
        },
        |s, t, db| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "b");
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert_eq!(entry.hash.unwrap(), hash_file(&s.join("file")).unwrap());
        },
        DeployType::Copy; "copy_reapply_source_changed"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |s, _| {
            fs::write(s.join("file"), "b").unwrap();
        },
        |s, t, db| {
            assert_eq!(fs::read_link(t.join("file")).unwrap(), s.join("file"));
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Symlink);
            assert!(entry.hash.is_none());
        },
        DeployType::Symlink; "symlink_reapply_source_changed"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, t| {
            fs::write(t.join("file"), "external").unwrap();
        },
        |s, t, db| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "external");
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert_eq!(entry.hash.unwrap(), hash_file(&s.join("file")).unwrap());
        },
        DeployType::Copy; "copy_reapply_external_modification_skip"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            fs::write(t.join("file"), "external").unwrap();
        },
        |s, t, db| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "a");
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert_eq!(entry.hash.unwrap(), hash_file(&s.join("file")).unwrap());
        },
        DeployType::Copy; "copy_reapply_external_modification_overwrite"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, t| {
            let _ = fs::remove_file(t.join("file"));
            fs::write(t.join("file"), "external").unwrap();
        },
        |_, t, db| {
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "external");
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Symlink);
            assert!(entry.hash.is_none());
        },
        DeployType::Symlink; "symlink_reapply_target_replaced_with_file_skip"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, t| {
            let _ = fs::remove_file(t.join("file"));
            unix_fs::symlink(Path::new("/wrong"), t.join("file")).unwrap();
        },
        |s, t, db| {
            assert_eq!(fs::read_link(t.join("file")).unwrap(), Path::new("/wrong"));
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.source_path, s.join("file"));
            assert_eq!(entry.deploy_type, DeployType::Symlink);
            assert!(entry.hash.is_none());
        },
        DeployType::Symlink; "symlink_reapply_target_replaced_with_wrong_symlink_skip"
    )]
    #[test_case(
        |s, _| {
            unix_fs::symlink(Path::new("/a"), s.join("file")).unwrap();
        },
        |_, _| {},
        |_, t, db| {
            assert!(t.join("file").is_symlink());
            assert_eq!(fs::read_link(t.join("file")).unwrap(), Path::new("/a"));
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert!(entry.hash.is_none());
            assert_eq!(entry.symlink_target, Some(PathBuf::from("/a")));
        },
        DeployType::Copy; "copy_reapply_symlink_source_identical"
    )]
    #[test_case(
        |s, _| {
            unix_fs::symlink(Path::new("/a"), s.join("file")).unwrap();
        },
        |s, _| {
            let _ = fs::remove_file(s.join("file"));
            unix_fs::symlink(Path::new("/b"), s.join("file")).unwrap();
        },
        |_, t, db| {
            assert!(t.join("file").is_symlink());
            assert_eq!(fs::read_link(t.join("file")).unwrap(), Path::new("/b"));
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert!(entry.hash.is_none());
            assert_eq!(entry.symlink_target, Some(PathBuf::from("/b")));
        },
        DeployType::Copy; "copy_reapply_symlink_source_changed"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, t| {
            let _ = fs::remove_file(t.join("file"));
            unix_fs::symlink(Path::new("/evil"), t.join("file")).unwrap();
        },
        |s, t, db| {
            assert!(t.join("file").is_symlink());
            assert_eq!(fs::read_link(t.join("file")).unwrap(), Path::new("/evil"));
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert_eq!(entry.hash.unwrap(), hash_file(&s.join("file")).unwrap());
        },
        DeployType::Copy; "copy_reapply_target_type_changed_skip"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("file"), "a").unwrap();
        },
        |_, t| {
            PROMPT_SELECTION.set(CollisionOptions::Overwrite);
            let _ = fs::remove_file(t.join("file"));
            unix_fs::symlink(Path::new("/evil"), t.join("file")).unwrap();
        },
        |_, t, db| {
            assert!(!t.join("file").is_symlink());
            assert_eq!(fs::read_to_string(t.join("file")).unwrap(), "a");
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Copy);
            assert!(entry.hash.is_some());
        },
        DeployType::Copy; "copy_reapply_target_type_changed_overwrite"
    )]
    #[test_case(
        |s, _| {
            fs::write(s.join("dotrift_data.toml"), r#"[variable]
name = "world""#).unwrap();
            fs::write(s.join("file"), "Hello {{ name }}").unwrap();
        },
        |_, _| {},
        |_, t, db| {
            let rendered = fs::read_to_string(t.join("file")).unwrap();
            assert_eq!(rendered, "Hello world");
            let entry = db.get_entry(&t.join("file")).unwrap().unwrap();
            assert_eq!(entry.deploy_type, DeployType::Tmpl);
            assert_eq!(entry.hash.unwrap(), hash_file(&t.join("file")).unwrap());
        },
        DeployType::Tmpl; "tmpl_reapply_identical"
    )]
    fn test_apply_twice(
        setup1: impl FnOnce(&Path, &Path),
        setup2: impl FnOnce(&Path, &Path),
        assert: impl FnOnce(&Path, &Path, &Db),
        deploy_type: DeployType,
    ) {
        let (temp_dir, source_dir, target_dir) = setup_test(
            r#""" = """#,
            "",
            match deploy_type {
                DeployType::Symlink => "",
                DeployType::Copy => r#""**/*" = { type = "copy" }"#,
                DeployType::Tmpl => r#""**/*" = { type = "tmpl" }"#,
            },
            false,
        );
        setup1(&source_dir, &target_dir);
        let db_path = temp_dir.path().join("db");
        mock_apply(&source_dir, &target_dir, &db_path, FLAGS).unwrap();
        setup2(&source_dir, &target_dir);
        mock_apply(&source_dir, &target_dir, &db_path, FLAGS).unwrap();
        let db = Db::init(&db_path).unwrap();
        assert(&source_dir, &target_dir, &db);
    }

    #[test]
    fn test_deploy_permission() {
        let (temp_dir, source_dir, target_dir) = setup_test(
            r#""" = """#,
            "",
            r#""**/*" = { type = "copy", mode = "123" }"#,
            false,
        );
        fs::write(source_dir.join("file"), "").unwrap();
        mock_apply(&source_dir, &target_dir, &temp_dir.path().join("db"), FLAGS).unwrap();
        assert_eq!(
            target_dir
                .join("file")
                .metadata()
                .unwrap()
                .permissions()
                .mode(),
            0o100123
        );
    }

    #[test_case(FLAGS; "creates_file")]
    #[test_case(ApplyFlags { dry_run: true, ..FLAGS }; "dry_run_no_file")]
    fn test_apply_deploy(flags: ApplyFlags) {
        let (temp_dir, source_dir, target_dir) = setup_test(r#""" = """#, "", "", false);
        fs::write(source_dir.join("file"), "").unwrap();
        mock_apply(&source_dir, &target_dir, &temp_dir.path().join("db"), flags).unwrap();
        assert_eq!(target_dir.join("file").exists(), !flags.dry_run);
    }

    #[test_case(false; "removes_file")]
    #[test_case(true; "dry_run_preserves_file")]
    fn test_apply_clean_up(dry_run: bool) {
        let (temp_dir, source_dir, target_dir) = setup_test(r#""" = """#, "", "", false);
        fs::write(source_dir.join("file"), "").unwrap();
        mock_apply(&source_dir, &target_dir, &temp_dir.path().join("db"), FLAGS).unwrap();
        assert!(target_dir.join("file").exists());
        fs::write(source_dir.join("dotrift.toml"), "").unwrap();
        mock_apply(
            &source_dir,
            &target_dir,
            &temp_dir.path().join("db"),
            ApplyFlags {
                dry_run,
                clean_up: true,
                ..FLAGS
            },
        )
        .unwrap();
        assert_eq!(target_dir.join("file").exists(), dry_run);
    }

    #[test]
    fn test_dry_run_print_snapshot() {
        let (temp_dir, source_dir, target_dir) =
            setup_test(r#""" = """#, "", r#""subdir/*" = { type = "copy" }"#, true);
        mock_apply(
            &source_dir,
            &target_dir,
            &temp_dir.path().join("db"),
            ApplyFlags {
                dry_run: true,
                clean_up: false,
                prune_empty_dirs: false,
            },
        )
        .unwrap();

        assert_captured_output("apply_dry_run", temp_dir.path())
    }

    #[test]
    fn test_clean_up_dry_run_print_snapshot() {
        let (temp_dir, source_dir, target_dir) = setup_test(r#""" = """#, "", "", true);
        mock_apply(
            &source_dir,
            &target_dir,
            &temp_dir.path().join("db"),
            ApplyFlags {
                dry_run: false,
                clean_up: false,
                prune_empty_dirs: false,
            },
        )
        .unwrap();

        fs::write(source_dir.join("dotrift.toml"), "").unwrap();

        mock_apply(
            &source_dir,
            &target_dir,
            &temp_dir.path().join("db"),
            ApplyFlags {
                dry_run: true,
                clean_up: true,
                prune_empty_dirs: false,
            },
        )
        .unwrap();

        assert_captured_output("apply_clean_up_dry_run", temp_dir.path())
    }
}
