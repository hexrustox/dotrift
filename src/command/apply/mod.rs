use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use glob::Pattern;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use miette::{Context, Result, miette};

use crate::{
    cli::{ApplyFlags, GlobalFlags},
    command::{
        tree::build_tree,
        util::{GLOB_OPTION, StripPrefixOrSelf, clean_up, resolve_portal_entries, resolve_target},
    },
    config::{Config, DeployType, FileMode, Rules},
    db::Db,
    global_config::GlobalConfig,
    output,
    templater::{data, function::BuiltinFunctions},
};

use deploy::traverse_tree;
use dry_run::print_tree;

mod deploy;
mod dry_run;

#[derive(Default, Debug, PartialEq)]
pub struct PortalEntry {
    pub source: PathBuf,
    pub deploy_type: DeployType,
    pub mode: Option<FileMode>,
}

impl PortalEntry {
    pub fn new(source: PathBuf) -> Self {
        Self {
            source,
            deploy_type: DeployType::default(),
            mode: None,
        }
    }
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
            insert_portal_entry(&mut portal_entries, target_path, source_path, source_dir)
        },
    )?;

    Ok(portal_entries)
}

fn insert_portal_entry(
    portal_entries: &mut HashMap<PathBuf, PortalEntry>,
    target_path: PathBuf,
    source_path: PathBuf,
    source_dir: &Path,
) -> Result<()> {
    if target_path.starts_with(source_dir) {
        return Err(miette!(
            "target path `{}` is inside source directory `{}`",
            target_path.display(),
            source_dir.display()
        ));
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::fs;
    use test_case::test_case;

    use crate::command::util::tests::setup_test;

    thread_local! {
        pub static CHECK_MANAGED: RefCell<bool> = const { RefCell::new(false) };
    }

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

    #[test_case(r#""file" = "b/file""# => panics "inside source"; "target_inside_source_literal")]
    #[test_case(r#""*" = "b""# => panics "inside source"; "target_inside_source_glob")]
    #[test_case(r#""file" = "other""# => (); "target_not_inside_source")]
    fn test_target_not_in_source(portal: &str) {
        let temp_dir = tempfile::tempdir().unwrap();
        let target_dir = temp_dir.path().join("a");
        let source_dir = target_dir.join("b");
        fs::create_dir_all(&source_dir).unwrap();
        fs::write(source_dir.join("file"), "").unwrap();
        let config = format!("[portal]\n{portal}");
        fs::write(source_dir.join("dotrift.toml"), config).unwrap();

        let config = Config::read(&source_dir).unwrap();
        let ignore_matcher = build_ignore(&config.ignore, &target_dir).unwrap();
        resolve_portals(&source_dir, &target_dir, &config.portal, &ignore_matcher).unwrap();
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

    pub const FLAGS: ApplyFlags = ApplyFlags {
        dry_run: false,
        clean_up: false,
        prune_empty_dirs: false,
    };

    pub fn mock_apply(
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
}
