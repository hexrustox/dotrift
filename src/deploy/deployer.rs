//! Deploys decided entries to the filesystem and owns all filesystem mutation.
//!
//! The orchestrator (`commands::apply`) decides *what* to do via
//! `reconcile::decide`; this module performs the effect and owns the shared
//! `remove_path` primitive. Obstruction prompts are delegated to an
//! `ObstructionResolver`; no paths cross that seam.

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, BufWriter},
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

use miette::{Result, WrapErr, miette};
use templater::value::Value;

use super::obstruction::{ObstructionResolver, ResolveAction};
use super::reconcile::{Decision, decide};
use crate::{
    commands::apply::ApplyOptions,
    config::{self, DeployType, GlobalConfig},
    platform::prettify_path,
    render::{RenderRegistry, render_template_to},
    report::{Outcome, Reporter},
    state::{Fingerprint, HashWriter, Kind, StateDatabase, StateRecord, is_managed},
};

/// Latch for the `replace all` obstruction choice: once enabled, every
/// upcoming obstruction is replaced without prompting.
///
/// Documented as preview-only in dry-run (ADR-0014): dry-run never enables it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ReplaceLatch {
    replace_all: bool,
}

impl ReplaceLatch {
    pub(crate) fn is_enabled(&self) -> bool {
        self.replace_all
    }

    pub(crate) fn enable(&mut self) {
        self.replace_all = true;
    }
}

/// Single word table mapping a reconcile `Decision` to its report
/// `(Outcome, word)`, shared by dry-run and real-run rendering.
pub(crate) fn describe_decision(decision: &Decision) -> (Outcome, &'static str) {
    match decision {
        Decision::Deployed => (Outcome::Deployed, "deployed"),
        Decision::Replaced { .. } => (Outcome::Replaced, "replaced"),
        Decision::Prompt(_) => (Outcome::Obstruction, "obstruction"),
    }
}

/// Outcome of a single `deploy_one` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeployOutcome {
    Deployed,
    Replaced,
    Skipped,
    Cancelled,
}

/// Performs decided deployments. Single owner of `remove_path`.
pub(crate) struct Deployer<'a> {
    database: &'a StateDatabase,
    target_root: &'a Path,
    registry: &'a mut RenderRegistry,
    global_config: &'a GlobalConfig,
    interaction: &'a dyn ObstructionResolver,
    latch: &'a mut ReplaceLatch,
}

impl<'a> Deployer<'a> {
    pub(crate) fn new(
        database: &'a StateDatabase,
        target_root: &'a Path,
        registry: &'a mut RenderRegistry,
        global_config: &'a GlobalConfig,
        interaction: &'a dyn ObstructionResolver,
        latch: &'a mut ReplaceLatch,
    ) -> Self {
        Self {
            database,
            target_root,
            registry,
            global_config,
            interaction,
            latch,
        }
    }

    pub(crate) fn deploy_one(
        &mut self,
        entry: &config::DeploymentEntry,
        context: &HashMap<String, Value>,
    ) -> Result<DeployOutcome> {
        if !fs::metadata(&entry.source_path)
            .map_err(|error| miette!(error))?
            .is_file()
        {
            return Err(miette!(
                "source path `{}` is no longer a regular file",
                entry.source_path.display()
            ));
        }

        let mut replaced = false;
        match decide(
            self.database,
            self.target_root,
            entry,
            context,
            self.registry,
            self.latch.is_enabled(),
            self.global_config.replace_identical(),
        )? {
            Decision::Deployed => {}
            Decision::Replaced { remove } => {
                remove_path(self.database, &remove)?;
                replaced = true;
            }
            Decision::Prompt(obstruction) => match self.interaction.resolve_obstruction(
                entry,
                &obstruction,
                context,
                self.registry,
            )? {
                ResolveAction::Skip => return Ok(DeployOutcome::Skipped),
                ResolveAction::Cancel => return Ok(DeployOutcome::Cancelled),
                ResolveAction::Replace { latch_all } => {
                    if latch_all {
                        self.latch.enable();
                    }
                    remove_path(self.database, &obstruction)?;
                    replaced = true;
                }
            },
        }
        let parent = entry
            .target_path
            .parent()
            .ok_or_else(|| miette!("target path has no parent"))?;
        fs::create_dir_all(parent)
            .map_err(|error| miette!(error))
            .wrap_err("cannot create target parent directories")?;

        let record = match entry.deploy_type {
            DeployType::Symlink => {
                symlink(&entry.source_path, &entry.target_path)
                    .map_err(|error| miette!(error))
                    .wrap_err("cannot create target symlink")?;
                StateRecord {
                    target_path: entry.target_path.clone(),
                    source_path: entry.source_path.clone(),
                    kind: Kind::Symlink,
                    content_hash: None,
                }
            }
            DeployType::Copy | DeployType::Template => {
                let content_hash = write_deployed_file(entry, context, self.registry)?;
                StateRecord {
                    target_path: entry.target_path.clone(),
                    source_path: entry.source_path.clone(),
                    kind: Kind::File,
                    content_hash: Some(content_hash.into()),
                }
            }
        };
        self.database.put(&record)?;
        if let Some(mode) = entry.mode {
            fs::set_permissions(&entry.target_path, fs::Permissions::from_mode(mode.into()))
                .map_err(|error| miette!(error))
                .wrap_err("cannot apply target mode")?;
        }
        Ok(if replaced {
            DeployOutcome::Replaced
        } else {
            DeployOutcome::Deployed
        })
    }
}

/// Writes a copy or template entry straight to its target file, returning the
/// digest of the deployed bytes.
///
/// A template entry takes its rendered bytes from the render registry when it
/// is usable, copying them into the freshly created target; otherwise it
/// renders directly as the target is written. The file is created before
/// writing, so a failed copy or write leaves a partial target behind; it is
/// removed to keep the failure contract that the target is absent after a
/// failed deploy action. A render failure happens before the target is
/// created and leaves it absent.
fn write_deployed_file(
    entry: &config::DeploymentEntry,
    context: &HashMap<String, Value>,
    registry: &mut RenderRegistry,
) -> Result<Fingerprint> {
    if entry.deploy_type == DeployType::Template
        && let Some(rendered) = registry.ensure_rendered(&entry.source_path, context)?
    {
        let mut file = fs::File::create(&entry.target_path)
            .map_err(|error| miette!(error))
            .wrap_err("cannot write target file")?;
        let outcome = match fs::File::open(&rendered.path) {
            Ok(mut source) => io::copy(&mut source, &mut file)
                .map(|_| ())
                .map_err(|error| miette!(error))
                .wrap_err("cannot write target file"),
            Err(error) => Err(miette!(error).wrap_err("cannot read template render registry")),
        };
        if let Err(error) = outcome {
            let _ = fs::remove_file(&entry.target_path);
            return Err(error);
        }
        return Ok(rendered.digest);
    }
    let file = fs::File::create(&entry.target_path)
        .map_err(|error| miette!(error))
        .wrap_err("cannot write target file")?;
    let mut writer = HashWriter::new(BufWriter::new(file));
    let outcome = match entry.deploy_type {
        DeployType::Template => render_template_to(&entry.source_path, context, &mut writer),
        DeployType::Copy => {
            let mut source = fs::File::open(&entry.source_path)
                .map_err(|error| miette!(error))
                .wrap_err("cannot read copy source")?;
            io::copy(&mut source, &mut writer)
                .map_err(|error| miette!(error))
                .wrap_err("cannot write target file")?;
            Ok(())
        }
        DeployType::Symlink => Err(miette!("symlink entries do not deploy as files")),
    };
    let content_hash = writer.into_digest();
    if let Err(error) = outcome {
        let _ = fs::remove_file(&entry.target_path);
        return Err(error);
    }
    Ok(content_hash)
}

/// Removes stale paths: managed paths under the target root that are not part
/// of the desired deployment. Single owner of `remove_path` for the Relinquish
/// + prune-empty-dirs scan.
pub(crate) fn cleanup(
    database: &StateDatabase,
    target_root: &Path,
    desired: &HashSet<PathBuf>,
    options: ApplyOptions,
    report: &Reporter,
) -> Result<(usize, usize)> {
    let dry_run = options.dry_run;
    let mut removed = 0;
    let mut pruned = 0;
    let mut planned_removals = HashSet::new();
    let mut records = database.managed_paths()?;
    records.sort_by(|left, right| left.target_path.cmp(&right.target_path));
    for record in records {
        let path = &record.target_path;
        if path == target_root || !path.starts_with(target_root) || desired.contains(path) {
            continue;
        }
        let exists = match fs::symlink_metadata(path) {
            Ok(_) => true,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    || error.kind() == std::io::ErrorKind::NotADirectory =>
            {
                false
            }
            Err(error) => return Err(miette!(error).wrap_err("cannot inspect stale target")),
        };
        if !exists {
            if !dry_run {
                database.remove(path)?;
            }
            continue;
        }
        if !is_managed(&record)? {
            if !dry_run {
                database.remove(path)?;
            }
            continue;
        }
        if dry_run {
            planned_removals.insert(path.clone());
            report.outcome_line(format_args!(
                "{} {}",
                report.paint(Outcome::Removed, "removed"),
                prettify_path(path).display()
            ));
            continue;
        }
        remove_path(database, path)?;
        removed += 1;
        report.outcome_line(format_args!(
            "{} {}",
            report.paint(Outcome::Removed, "removed"),
            prettify_path(path).display()
        ));
        if options.prune_empty_dirs {
            pruned += prune_parents(target_root, path, report)?;
        }
    }
    if dry_run && options.prune_empty_dirs {
        report_dry_run_pruning(target_root, &planned_removals, report)?;
    }
    Ok((removed, pruned))
}

fn report_dry_run_pruning(
    target_root: &Path,
    removals: &HashSet<PathBuf>,
    report: &Reporter,
) -> Result<()> {
    let mut planned = removals.clone();
    let mut parents = removals
        .iter()
        .filter_map(|path| path.parent())
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    parents.sort();
    parents.dedup();
    for parent in parents {
        let mut current = Some(parent);
        while let Some(directory) = current {
            if directory == target_root || !directory.starts_with(target_root) {
                break;
            }
            if !would_be_empty(&directory, &planned)? {
                break;
            }
            report.outcome_line(format_args!(
                "{} {}",
                report.paint(Outcome::Pruned, "pruned"),
                prettify_path(&directory).display()
            ));
            planned.insert(directory.clone());
            current = directory.parent().map(Path::to_path_buf);
        }
    }
    Ok(())
}

fn would_be_empty(path: &Path, removals: &HashSet<PathBuf>) -> Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(miette!(error).wrap_err("cannot inspect prune directory")),
    };
    if !metadata.file_type().is_dir() {
        return Ok(false);
    }
    for child in fs::read_dir(path)
        .map_err(|error| miette!(error).wrap_err("cannot inspect prune directory"))?
    {
        let child = child
            .map_err(|error| miette!(error).wrap_err("cannot inspect prune directory"))?
            .path();
        if removals.contains(&child) {
            continue;
        }
        return Ok(false);
    }
    Ok(true)
}

fn prune_parents(target_root: &Path, removed_path: &Path, report: &Reporter) -> Result<usize> {
    let mut current = removed_path.parent();
    let mut count = 0;
    while let Some(parent) = current {
        if parent == target_root || !parent.starts_with(target_root) {
            break;
        }
        let metadata = match fs::symlink_metadata(parent) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(miette!(error).wrap_err("cannot inspect prune directory")),
        };
        if !metadata.file_type().is_dir() {
            break;
        }
        let mut children = fs::read_dir(parent)
            .map_err(|error| miette!(error).wrap_err("cannot inspect prune directory"))?;
        if children.next().is_some() {
            break;
        }
        fs::remove_dir(parent)
            .map_err(|error| miette!(error).wrap_err("cannot prune empty directory"))?;
        count += 1;
        report.outcome_line(format_args!(
            "{} {}",
            report.paint(Outcome::Pruned, "pruned"),
            prettify_path(parent).display()
        ));
        current = parent.parent();
    }
    Ok(count)
}

/// Removes `path` from the filesystem and drops its state record.
///
/// Directories are removed recursively, deepest-first in component order,
/// stopping at the first error. Symlinks are unlinked as links, never
/// followed. The state record is deleted after each completed removal.
pub(crate) fn remove_path(database: &StateDatabase, path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| miette!(error))?;
    if metadata.file_type().is_dir() {
        let mut children = fs::read_dir(path)
            .map_err(|error| miette!(error))?
            .map(|entry| {
                entry
                    .map(|entry| entry.path())
                    .map_err(|error| miette!(error))
            })
            .collect::<Result<Vec<_>>>()?;
        children.sort();
        for child in children {
            remove_path(database, &child)?;
        }
        fs::remove_dir(path).map_err(|error| miette!(error))?;
    } else {
        fs::remove_file(path).map_err(|error| miette!(error))?;
    }
    database.remove(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use tempfile::tempdir;
    use test_case::test_case;

    #[test]
    fn latch_enable_stays_enabled() {
        let mut latch = ReplaceLatch::default();
        assert!(!latch.is_enabled());
        latch.enable();
        assert!(latch.is_enabled());
        latch.enable();
        assert!(latch.is_enabled());
    }

    #[test]
    fn describe_decision_maps_three_arms() {
        assert_eq!(
            describe_decision(&Decision::Deployed),
            (Outcome::Deployed, "deployed")
        );
        assert_eq!(
            describe_decision(&Decision::Replaced {
                remove: PathBuf::from("/tmp/x")
            }),
            (Outcome::Replaced, "replaced")
        );
        assert_eq!(
            describe_decision(&Decision::Prompt(PathBuf::from("/tmp/x"))),
            (Outcome::Obstruction, "obstruction")
        );
    }

    #[test_case(|_t| vec![] => true ; "empty_directory_reports_empty")]
    #[test_case(|t| {
        fs::write(t.join("file"), "content").unwrap();
        vec![]
    } => false ; "directory_with_unremoved_file_reports_not_empty")]
    #[test_case(|t| {
        fs::write(t.join("a"), "content").unwrap();
        fs::write(t.join("b"), "content").unwrap();
        vec![t.join("a"), t.join("b")]
    } => true ; "directory_with_all_files_removed_reports_empty")]
    #[test_case(|t| {
        fs::write(t.join("a"), "content").unwrap();
        fs::write(t.join("b"), "content").unwrap();
        vec![t.join("a")]
    } => false ; "directory_with_some_files_kept_reports_not_empty")]
    #[test_case(|t| {
        fs::create_dir_all(t.join("sub")).unwrap();
        fs::write(t.join("sub/file"), "content").unwrap();
        vec![t.join("sub")]
    } => true ; "directory_with_subdir_removed_reports_empty")]
    fn reports_would_be_empty_for(setup: impl Fn(&Path) -> Vec<PathBuf>) -> bool {
        let dir = tempdir().expect("cannot create temp dir");
        would_be_empty(dir.path(), &HashSet::from_iter(setup(dir.path()))).unwrap()
    }

    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a")).unwrap();
            t.join("a/file")
        },
        |t| assert!(!t.join("a").exists())
        ; "empty_parent_pruned"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a/b")).unwrap();
            t.join("a/b/file")
        },
        |t| {
            assert!(!t.join("a/b").exists());
            assert!(!t.join("a").exists());
        }
        ; "nested_empty_parents_pruned_up_to_root"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a/b")).unwrap();
            fs::write(t.join("a/keep"), "content").unwrap();
            t.join("a/b/file")
        },
        |t| {
            assert!(!t.join("a/b").exists());
            assert!(t.join("a").exists());
            assert!(t.join("a/keep").exists());
        }
        ; "pruning_stops_at_non_empty_parent"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("a"), "occupied").unwrap();
            t.join("a/x")
        },
        |t| assert_eq!(fs::read_to_string(t.join("a")).unwrap(), "occupied")
        ; "non_directory_parent_stops_pruning"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("real")).unwrap();
            std::os::unix::fs::symlink(t.join("real"), t.join("link")).unwrap();
            t.join("link/file")
        },
        |t| {
            assert!(fs::symlink_metadata(t.join("link"))
                .unwrap()
                .file_type()
                .is_symlink());
            assert!(t.join("real").exists());
        }
        ; "symlink_parent_stops_pruning"
    )]
    fn prunes_empty_parents_for(setup: impl Fn(&Path) -> PathBuf, assert: impl Fn(&Path)) {
        let dir = tempdir().expect("cannot create temp dir");
        prune_parents(
            dir.path(),
            &setup(dir.path()),
            &crate::report::Reporter::always(false),
        )
        .unwrap_or_else(|error| panic!("{error}"));
        assert(dir.path());
    }

    struct FakeResolver {
        actions: std::cell::RefCell<Vec<ResolveAction>>,
        calls: std::cell::RefCell<usize>,
    }

    impl FakeResolver {
        fn once(action: ResolveAction) -> Self {
            Self {
                actions: std::cell::RefCell::new(vec![action]),
                calls: std::cell::RefCell::new(0),
            }
        }

        fn calls(&self) -> usize {
            *self.calls.borrow()
        }
    }

    impl ObstructionResolver for FakeResolver {
        fn resolve_obstruction(
            &self,
            _entry: &config::DeploymentEntry,
            _obstruction: &Path,
            _context: &HashMap<String, Value>,
            _registry: &mut RenderRegistry,
        ) -> Result<ResolveAction> {
            *self.calls.borrow_mut() += 1;
            Ok(self
                .actions
                .borrow_mut()
                .pop()
                .expect("resolver actions exhausted"))
        }
    }

    struct PanickingResolver;

    impl ObstructionResolver for PanickingResolver {
        fn resolve_obstruction(
            &self,
            _entry: &config::DeploymentEntry,
            _obstruction: &Path,
            _context: &HashMap<String, Value>,
            _registry: &mut RenderRegistry,
        ) -> Result<ResolveAction> {
            panic!("resolver must not be called for managed paths");
        }
    }

    fn test_harness() -> (
        tempfile::TempDir,
        tempfile::TempDir,
        tempfile::TempDir,
        tempfile::TempDir,
    ) {
        (
            tempdir().expect("source temp dir"),
            tempdir().expect("target temp dir"),
            tempdir().expect("state temp dir"),
            tempdir().expect("registry temp dir"),
        )
    }

    fn copy_entry(source: &Path, target: &Path) -> config::DeploymentEntry {
        config::DeploymentEntry {
            source_path: source.to_path_buf(),
            target_path: target.to_path_buf(),
            deploy_type: DeployType::Copy,
            mode: None,
        }
    }

    #[test]
    fn deploy_one_deploys_missing_target() {
        use crate::platform::Environment;

        let (source, target, state, registry_root) = test_harness();
        fs::write(source.path().join("file.txt"), "new").unwrap();
        let database = StateDatabase::open_at(state.path()).unwrap();
        let env = Environment::test_root(registry_root.path());
        let mut registry = RenderRegistry::acquire(&env, false);
        let global_config = GlobalConfig::default();
        let interaction = PanickingResolver;
        let mut latch = ReplaceLatch::default();
        let mut deployer = Deployer::new(
            &database,
            target.path(),
            &mut registry,
            &global_config,
            &interaction,
            &mut latch,
        );
        let entry = copy_entry(
            &source.path().join("file.txt"),
            &target.path().join("target.txt"),
        );

        let outcome = deployer.deploy_one(&entry, &HashMap::new()).unwrap();
        assert_eq!(outcome, DeployOutcome::Deployed);
        assert_eq!(
            fs::read_to_string(target.path().join("target.txt")).unwrap(),
            "new"
        );
    }

    #[test]
    fn deploy_one_replaces_managed_path_without_prompting() {
        use crate::{
            platform::Environment,
            state::{Kind, StateRecord, hash_bytes},
        };

        let (source, target, state, registry_root) = test_harness();
        fs::write(source.path().join("file.txt"), "v1").unwrap();
        fs::write(target.path().join("target.txt"), "v1").unwrap();
        let database = StateDatabase::open_at(state.path()).unwrap();
        database
            .put(&StateRecord {
                target_path: target.path().join("target.txt"),
                source_path: source.path().join("file.txt"),
                kind: Kind::File,
                content_hash: Some(hash_bytes(b"v1").into()),
            })
            .unwrap();
        let env = Environment::test_root(registry_root.path());
        let mut registry = RenderRegistry::acquire(&env, false);
        let global_config = GlobalConfig::default();
        let interaction = PanickingResolver;
        let mut latch = ReplaceLatch::default();
        let mut deployer = Deployer::new(
            &database,
            target.path(),
            &mut registry,
            &global_config,
            &interaction,
            &mut latch,
        );
        let entry = copy_entry(
            &source.path().join("file.txt"),
            &target.path().join("target.txt"),
        );

        let outcome = deployer.deploy_one(&entry, &HashMap::new()).unwrap();
        assert_eq!(outcome, DeployOutcome::Replaced);
    }

    #[test]
    fn deploy_one_skip_and_cancel() {
        use crate::platform::Environment;

        for (action, expected) in [
            (ResolveAction::Skip, DeployOutcome::Skipped),
            (ResolveAction::Cancel, DeployOutcome::Cancelled),
        ] {
            let (source, target, state, registry_root) = test_harness();
            fs::write(source.path().join("file.txt"), "new").unwrap();
            fs::write(target.path().join("target.txt"), "old").unwrap();
            let database = StateDatabase::open_at(state.path()).unwrap();
            let env = Environment::test_root(registry_root.path());
            let mut registry = RenderRegistry::acquire(&env, false);
            let global_config = GlobalConfig::default();
            let interaction = FakeResolver::once(action);
            let mut latch = ReplaceLatch::default();
            let mut deployer = Deployer::new(
                &database,
                target.path(),
                &mut registry,
                &global_config,
                &interaction,
                &mut latch,
            );
            let entry = copy_entry(
                &source.path().join("file.txt"),
                &target.path().join("target.txt"),
            );

            let outcome = deployer.deploy_one(&entry, &HashMap::new()).unwrap();
            assert_eq!(outcome, expected);
            assert_eq!(interaction.calls(), 1);
            assert_eq!(
                fs::read_to_string(target.path().join("target.txt")).unwrap(),
                "old"
            );
        }
    }

    #[test]
    fn deploy_one_latch_suppresses_second_prompt() {
        use crate::platform::Environment;

        let (source, target, state, registry_root) = test_harness();
        fs::write(source.path().join("a.txt"), "new-a").unwrap();
        fs::write(source.path().join("b.txt"), "new-b").unwrap();
        fs::write(target.path().join("a.txt"), "old-a").unwrap();
        fs::write(target.path().join("b.txt"), "old-b").unwrap();
        let database = StateDatabase::open_at(state.path()).unwrap();
        let env = Environment::test_root(registry_root.path());
        let mut registry = RenderRegistry::acquire(&env, false);
        let global_config = GlobalConfig::default();
        let interaction = FakeResolver {
            actions: std::cell::RefCell::new(vec![ResolveAction::Replace { latch_all: true }]),
            calls: std::cell::RefCell::new(0),
        };
        let mut latch = ReplaceLatch::default();
        let first = copy_entry(&source.path().join("a.txt"), &target.path().join("a.txt"));
        let second = copy_entry(&source.path().join("b.txt"), &target.path().join("b.txt"));
        {
            let mut deployer = Deployer::new(
                &database,
                target.path(),
                &mut registry,
                &global_config,
                &interaction,
                &mut latch,
            );
            assert_eq!(
                deployer.deploy_one(&first, &HashMap::new()).unwrap(),
                DeployOutcome::Replaced
            );
            assert_eq!(
                deployer.deploy_one(&second, &HashMap::new()).unwrap(),
                DeployOutcome::Replaced
            );
        }
        assert_eq!(interaction.calls(), 1);
        assert!(latch.is_enabled());
    }

    #[test]
    fn deploy_one_template_failure_leaves_target_absent() {
        use crate::platform::Environment;

        let (source, target, state, registry_root) = test_harness();
        fs::write(source.path().join("bad.txt"), "{{ unclosed").unwrap();
        let database = StateDatabase::open_at(state.path()).unwrap();
        let env = Environment::test_root(registry_root.path());
        let mut registry = RenderRegistry::acquire(&env, false);
        let global_config = GlobalConfig::default();
        let interaction = PanickingResolver;
        let mut latch = ReplaceLatch::default();
        let mut deployer = Deployer::new(
            &database,
            target.path(),
            &mut registry,
            &global_config,
            &interaction,
            &mut latch,
        );
        let entry = config::DeploymentEntry {
            source_path: source.path().join("bad.txt"),
            target_path: target.path().join("target.txt"),
            deploy_type: DeployType::Template,
            mode: None,
        };

        assert!(deployer.deploy_one(&entry, &HashMap::new()).is_err());
        assert!(!target.path().join("target.txt").exists());
    }
}
