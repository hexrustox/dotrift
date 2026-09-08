//! The reconcile decision: what `apply` does with one desired entry before
//! any filesystem effect.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use miette::{Result, miette};
use templater::value::Value;

use crate::{
    config::{DeployType, DeploymentEntry},
    hash, managed,
    render_registry::RenderRegistry,
    state::StateDatabase,
};

/// What `apply` does with one entry before any filesystem effect: a missing
/// target deploys, an auto-replaceable occupant is removed and then deployed,
/// and anything else occupying the target path needs a user decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decision {
    /// The target path is free; deploy the entry.
    Deployed,
    /// `remove` occupies the target path and is replaced without prompting;
    /// remove it, then deploy.
    Replaced { remove: PathBuf },
    /// `obstruction` occupies the target path and must be decided by the user.
    Prompt(PathBuf),
}

/// Decides what `apply` does with `entry`: the three branches of ADR-0014
/// (missing target deploys, managed path is replaced, obstruction prompts)
/// plus the identical-obstruction auto-replace of ADR-0019, which is gated on
/// `replace_identical` and subsumed by `replace_all`. The obstruction check
/// walks the parent chain first, so an occupied parent short-circuits before
/// the target or its record are touched; rendering and hashing happen only
/// when the identical check needs them (ADR-0006), and a registry outage
/// means the check cannot establish identity (ADR-0017).
pub(crate) fn decide(
    database: &StateDatabase,
    target_root: &Path,
    entry: &DeploymentEntry,
    context: &HashMap<String, Value>,
    registry: &mut RenderRegistry,
    replace_all: bool,
    replace_identical: bool,
) -> Result<Decision> {
    if let Some(obstruction) = parent_obstruction(target_root, &entry.target_path)? {
        return Ok(if replace_all {
            Decision::Replaced {
                remove: obstruction,
            }
        } else {
            Decision::Prompt(obstruction)
        });
    }
    let existed = match fs::symlink_metadata(&entry.target_path) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(miette!(error).wrap_err(format!(
                "cannot inspect target `{}`",
                entry.target_path.display()
            )));
        }
    };
    if !existed {
        return Ok(Decision::Deployed);
    }
    let managed = match database.record(&entry.target_path)? {
        Some(record) => managed::is_managed(&record)?,
        None => false,
    };
    if managed
        || replace_all
        || (replace_identical && is_identical_obstruction(entry, context, registry))
    {
        return Ok(Decision::Replaced {
            remove: entry.target_path.clone(),
        });
    }
    Ok(Decision::Prompt(entry.target_path.clone()))
}

/// Whether the entry's own target path is an *identical obstruction*: for a
/// symlink deploy, a symlink whose link target equals the source path; for a
/// file deploy, a path resolving to a regular file whose content fingerprint
/// equals the fingerprint of the bytes that would be deployed. Any failure to
/// read a path or obtain the rendered bytes means the check cannot establish
/// identity and the obstruction is treated as not identical.
fn is_identical_obstruction(
    entry: &DeploymentEntry,
    context: &HashMap<String, Value>,
    registry: &mut RenderRegistry,
) -> bool {
    match entry.deploy_type {
        DeployType::Symlink => {
            fs::read_link(&entry.target_path).is_ok_and(|link| link == entry.source_path)
        }
        DeployType::Copy => file_matches(&entry.target_path, &entry.source_path),
        DeployType::Template => {
            let Ok(Some(rendered)) = registry.ensure_rendered(&entry.source_path, context) else {
                return false;
            };
            file_matches_digest(&entry.target_path, &rendered.digest)
        }
    }
}

/// Whether `path` resolves, following symlinks, to a regular file holding the
/// same bytes as `source`. File mode is not part of the comparison.
fn file_matches(path: &Path, source: &Path) -> bool {
    hash::hash_file(source).is_ok_and(|source_hash| file_matches_digest(path, &source_hash))
}

/// Whether `path` resolves, following symlinks, to a regular file whose
/// content fingerprint equals `digest`.
fn file_matches_digest(path: &Path, digest: &str) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
        && hash::hash_file(path).is_ok_and(|hash| hash == digest)
}

fn parent_obstruction(target_root: &Path, target_path: &Path) -> Result<Option<PathBuf>> {
    let parent = target_path
        .parent()
        .ok_or_else(|| miette!("target path has no parent"))?;
    let relative = parent
        .strip_prefix(target_root)
        .map_err(|_| miette!("target path is outside target directory"))?;
    let mut current = target_root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::metadata(&current) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Ok(Some(current)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if fs::symlink_metadata(&current).is_ok() {
                    return Ok(Some(current));
                }
                return Ok(None);
            }
            Err(error) => {
                return Err(miette!(error).wrap_err(format!(
                    "cannot inspect target parent `{}`",
                    current.display()
                )));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::os::unix::fs::symlink;

    use tempfile::tempdir;
    use templater::value::Value;
    use test_case::test_case;

    use crate::{
        environment::Environment,
        hash::hash_bytes,
        state::{Kind, StateRecord},
    };

    use super::*;

    fn entry(source: &Path, target: &Path, deploy_type: DeployType) -> DeploymentEntry {
        DeploymentEntry {
            source_path: source.to_path_buf(),
            target_path: target.to_path_buf(),
            deploy_type,
            mode: None,
        }
    }

    fn no_context() -> HashMap<String, Value> {
        HashMap::new()
    }

    fn dry_registry(anchor: &Path) -> RenderRegistry {
        RenderRegistry::acquire(&Environment::test_root(anchor), true)
    }

    #[test_case(|t| t.join("file") => None ; "target_directly_below_root_reports_no_obstruction")]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a/b")).unwrap();
            t.join("a/b/file")
        } => None;
        "directory_parents_report_no_obstruction"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a")).unwrap();
            t.join("a/b/file")
        } => None;
        "missing_parent_component_reports_no_obstruction"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a")).unwrap();
            fs::write(t.join("a/b"), "occupied").unwrap();
            t.join("a/b/file")
        } => Some(PathBuf::from("a/b"));
        "file_parent_reported_as_obstruction"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a/b")).unwrap();
            fs::write(t.join("a/b/f"), "content").unwrap();
            std::os::unix::fs::symlink(t.join("a/b/f"), t.join("a/b/link")).unwrap();
            t.join("a/b/link/file")
        } => Some(PathBuf::from("a/b/link"));
        "symlink_to_file_parent_reported_as_obstruction"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a/real")).unwrap();
            std::os::unix::fs::symlink(t.join("a/real"), t.join("a/dirlink")).unwrap();
            t.join("a/dirlink/file")
        } => None;
        "symlink_to_directory_parent_reports_no_obstruction"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a")).unwrap();
            std::os::unix::fs::symlink(t.join("a/nowhere"), t.join("a/broken")).unwrap();
            t.join("a/broken/file")
        } => Some(PathBuf::from("a/broken"));
        "dangling_symlink_parent_reported_as_obstruction"
    )]
    #[test_case(|_t| PathBuf::from("/unrelated/nested/target") => panics "outside target directory" ; "target_outside_target_root_is_rejected")]
    #[test_case(|_t| PathBuf::from("/") => panics "no parent" ; "target_without_parent_is_rejected")]
    fn reports_parent_obstruction_for(setup: impl Fn(&Path) -> PathBuf) -> Option<PathBuf> {
        let dir = tempdir().expect("cannot create temp dir");
        parent_obstruction(dir.path(), &setup(dir.path()))
            .unwrap_or_else(|error| panic!("{error}"))
            .map(|path| path.strip_prefix(dir.path()).unwrap().to_path_buf())
    }

    #[test]
    fn decides_deployed_for_missing_target() {
        let source = tempdir().unwrap();
        let target = tempdir().unwrap();
        let state = tempdir().unwrap();
        fs::write(source.path().join("file.txt"), "new").unwrap();
        let database = StateDatabase::open_at(state.path()).unwrap();
        let env = Environment::test_root(state.path());
        let entry = entry(
            &source.path().join("file.txt"),
            &target.path().join("target.txt"),
            DeployType::Copy,
        );
        let mut registry = RenderRegistry::acquire(&env, true);

        let decision = decide(
            &database,
            target.path(),
            &entry,
            &no_context(),
            &mut registry,
            false,
            true,
        )
        .unwrap();

        assert_eq!(decision, Decision::Deployed);
    }

    #[test_case(DeployType::Copy ; "managed_file_is_replaced")]
    #[test_case(DeployType::Symlink ; "managed_symlink_is_replaced")]
    fn decides_replaced_for_managed_path(deploy_type: DeployType) {
        let source = tempdir().unwrap();
        let target = tempdir().unwrap();
        let state = tempdir().unwrap();
        let database = StateDatabase::open_at(state.path()).unwrap();
        let target_path = target.path().join("target.txt");
        let record = match deploy_type {
            DeployType::Copy => {
                fs::write(source.path().join("file.txt"), "v1").unwrap();
                fs::write(&target_path, "v1").unwrap();
                StateRecord {
                    target_path: target_path.clone(),
                    source_path: source.path().join("file.txt"),
                    kind: Kind::File,
                    content_hash: Some(hash_bytes(b"v1")),
                }
            }
            DeployType::Symlink => {
                fs::write(source.path().join("file.txt"), "v1").unwrap();
                symlink(source.path().join("file.txt"), &target_path).unwrap();
                StateRecord {
                    target_path: target_path.clone(),
                    source_path: source.path().join("file.txt"),
                    kind: Kind::Symlink,
                    content_hash: None,
                }
            }
            DeployType::Template => unreachable!("template is covered by its own tests"),
        };
        database.put(&record).unwrap();
        let entry = entry(&source.path().join("file.txt"), &target_path, deploy_type);
        let mut registry = dry_registry(state.path());

        let decision = decide(
            &database,
            target.path(),
            &entry,
            &no_context(),
            &mut registry,
            false,
            false,
        )
        .unwrap();

        assert_eq!(
            decision,
            Decision::Replaced {
                remove: target_path
            }
        );
    }

    #[test]
    fn decides_prompt_for_divergent_content_even_when_replace_identical() {
        let source = tempdir().unwrap();
        let target = tempdir().unwrap();
        let state = tempdir().unwrap();
        fs::write(source.path().join("file.txt"), "new").unwrap();
        fs::write(target.path().join("target.txt"), "old").unwrap();
        let database = StateDatabase::open_at(state.path()).unwrap();
        let entry = entry(
            &source.path().join("file.txt"),
            &target.path().join("target.txt"),
            DeployType::Copy,
        );
        let mut registry = dry_registry(state.path());

        let decision = decide(
            &database,
            target.path(),
            &entry,
            &no_context(),
            &mut registry,
            false,
            true,
        )
        .unwrap();

        assert_eq!(decision, Decision::Prompt(target.path().join("target.txt")));
    }

    #[test_case(true, |target: &Path| Decision::Replaced { remove: target.join("target.txt") } ; "identical_copy_is_replaced_when_replace_identical")]
    #[test_case(false, |target: &Path| Decision::Prompt(target.join("target.txt")) ; "identical_copy_prompts_without_replace_identical")]
    fn decides_for_identical_copy_with(
        replace_identical: bool,
        expected: impl Fn(&Path) -> Decision,
    ) {
        let source = tempdir().unwrap();
        let target = tempdir().unwrap();
        let state = tempdir().unwrap();
        fs::write(source.path().join("file.txt"), "same").unwrap();
        fs::write(target.path().join("target.txt"), "same").unwrap();
        let database = StateDatabase::open_at(state.path()).unwrap();
        let entry = entry(
            &source.path().join("file.txt"),
            &target.path().join("target.txt"),
            DeployType::Copy,
        );
        let mut registry = dry_registry(state.path());

        let decision = decide(
            &database,
            target.path(),
            &entry,
            &no_context(),
            &mut registry,
            false,
            replace_identical,
        )
        .unwrap();

        assert_eq!(decision, expected(target.path()));
    }

    #[test]
    fn decides_replaced_for_any_existing_target_under_replace_all() {
        let source = tempdir().unwrap();
        let target = tempdir().unwrap();
        let state = tempdir().unwrap();
        fs::write(source.path().join("file.txt"), "new").unwrap();
        fs::write(target.path().join("target.txt"), "old").unwrap();
        let database = StateDatabase::open_at(state.path()).unwrap();
        let entry = entry(
            &source.path().join("file.txt"),
            &target.path().join("target.txt"),
            DeployType::Copy,
        );
        let mut registry = dry_registry(state.path());

        let decision = decide(
            &database,
            target.path(),
            &entry,
            &no_context(),
            &mut registry,
            true,
            false,
        )
        .unwrap();

        assert_eq!(
            decision,
            Decision::Replaced {
                remove: target.path().join("target.txt")
            }
        );
    }

    #[test_case(false, |root: &Path| Decision::Prompt(root.join("a")) ; "parent_obstruction_prompts")]
    #[test_case(true, |root: &Path| Decision::Replaced { remove: root.join("a") } ; "parent_obstruction_is_replaced_under_replace_all")]
    fn decides_for_parent_obstruction_with(
        replace_all: bool,
        expected: impl Fn(&Path) -> Decision,
    ) {
        let source = tempdir().unwrap();
        let target = tempdir().unwrap();
        let state = tempdir().unwrap();
        fs::write(source.path().join("file.txt"), "new").unwrap();
        fs::write(target.path().join("a"), "occupied").unwrap();
        let database = StateDatabase::open_at(state.path()).unwrap();
        let entry = entry(
            &source.path().join("file.txt"),
            &target.path().join("a/b.txt"),
            DeployType::Copy,
        );
        let mut registry = dry_registry(state.path());

        let decision = decide(
            &database,
            target.path(),
            &entry,
            &no_context(),
            &mut registry,
            replace_all,
            true,
        )
        .unwrap();

        assert_eq!(decision, expected(target.path()));
    }

    #[test]
    fn decides_replaced_for_identical_template_rendered_into_registry() {
        let source = tempdir().unwrap();
        let target = tempdir().unwrap();
        let state = tempdir().unwrap();
        let registry_root = tempdir().unwrap();
        let env = Environment::test_root(registry_root.path());
        fs::write(source.path().join("greeting.txt"), "{{ message }}\n").unwrap();
        fs::write(target.path().join("target.txt"), "hello\n").unwrap();
        let database = StateDatabase::open_at(state.path()).unwrap();
        let entry = entry(
            &source.path().join("greeting.txt"),
            &target.path().join("target.txt"),
            DeployType::Template,
        );
        let context = HashMap::from([("message".to_string(), Value::Str("hello".into()))]);
        let mut registry = RenderRegistry::acquire(&env, false);

        let decision = decide(
            &database,
            target.path(),
            &entry,
            &context,
            &mut registry,
            false,
            true,
        )
        .unwrap();

        assert_eq!(
            decision,
            Decision::Replaced {
                remove: target.path().join("target.txt")
            }
        );
    }

    #[test]
    fn decides_prompt_for_divergent_template() {
        let source = tempdir().unwrap();
        let target = tempdir().unwrap();
        let state = tempdir().unwrap();
        let registry_root = tempdir().unwrap();
        let env = Environment::test_root(registry_root.path());
        fs::write(source.path().join("greeting.txt"), "{{ message }}\n").unwrap();
        fs::write(target.path().join("target.txt"), "stale\n").unwrap();
        let database = StateDatabase::open_at(state.path()).unwrap();
        let entry = entry(
            &source.path().join("greeting.txt"),
            &target.path().join("target.txt"),
            DeployType::Template,
        );
        let context = HashMap::from([("message".to_string(), Value::Str("hello".into()))]);
        let mut registry = RenderRegistry::acquire(&env, false);

        let decision = decide(
            &database,
            target.path(),
            &entry,
            &context,
            &mut registry,
            false,
            true,
        )
        .unwrap();

        assert_eq!(decision, Decision::Prompt(target.path().join("target.txt")));
    }

    #[test]
    fn decides_prompt_for_identical_template_when_registry_is_unavailable() {
        let source = tempdir().unwrap();
        let target = tempdir().unwrap();
        let state = tempdir().unwrap();
        fs::write(source.path().join("greeting.txt"), "{{ message }}\n").unwrap();
        fs::write(target.path().join("target.txt"), "hello\n").unwrap();
        let database = StateDatabase::open_at(state.path()).unwrap();
        let entry = entry(
            &source.path().join("greeting.txt"),
            &target.path().join("target.txt"),
            DeployType::Template,
        );
        let context = HashMap::from([("message".to_string(), Value::Str("hello".into()))]);
        let mut registry = dry_registry(state.path());

        let decision = decide(
            &database,
            target.path(),
            &entry,
            &context,
            &mut registry,
            false,
            true,
        )
        .unwrap();

        assert_eq!(decision, Decision::Prompt(target.path().join("target.txt")));
    }

    #[test]
    fn decides_replaced_for_symlink_resolving_to_identical_file_when_replace_identical() {
        let source = tempdir().unwrap();
        let target = tempdir().unwrap();
        let state = tempdir().unwrap();
        fs::write(source.path().join("file.txt"), "new").unwrap();
        fs::write(target.path().join("other.txt"), "new").unwrap();
        symlink(
            target.path().join("other.txt"),
            target.path().join("target.txt"),
        )
        .unwrap();
        let database = StateDatabase::open_at(state.path()).unwrap();
        let entry = entry(
            &source.path().join("file.txt"),
            &target.path().join("target.txt"),
            DeployType::Copy,
        );
        let mut registry = dry_registry(state.path());

        let decision = decide(
            &database,
            target.path(),
            &entry,
            &no_context(),
            &mut registry,
            false,
            true,
        )
        .unwrap();

        assert_eq!(
            decision,
            Decision::Replaced {
                remove: target.path().join("target.txt")
            }
        );
    }

    #[test]
    fn decides_prompt_for_directory_target_even_when_inner_content_matches() {
        let source = tempdir().unwrap();
        let target = tempdir().unwrap();
        let state = tempdir().unwrap();
        fs::write(source.path().join("file.txt"), "same").unwrap();
        fs::create_dir(target.path().join("target.txt")).unwrap();
        fs::write(target.path().join("target.txt/inner"), "same").unwrap();
        let database = StateDatabase::open_at(state.path()).unwrap();
        let entry = entry(
            &source.path().join("file.txt"),
            &target.path().join("target.txt"),
            DeployType::Copy,
        );
        let mut registry = dry_registry(state.path());

        let decision = decide(
            &database,
            target.path(),
            &entry,
            &no_context(),
            &mut registry,
            false,
            true,
        )
        .unwrap();

        assert_eq!(decision, Decision::Prompt(target.path().join("target.txt")));
    }

    #[test]
    fn decides_replaced_for_previously_managed_file_matching_current_content() {
        let source = tempdir().unwrap();
        let target = tempdir().unwrap();
        let state = tempdir().unwrap();
        fs::write(source.path().join("file.txt"), "v2").unwrap();
        fs::write(target.path().join("target.txt"), "v2").unwrap();
        let database = StateDatabase::open_at(state.path()).unwrap();
        database
            .put(&StateRecord {
                target_path: target.path().join("target.txt"),
                source_path: source.path().join("file.txt"),
                kind: Kind::File,
                content_hash: Some(hash_bytes(b"v2")),
            })
            .unwrap();
        let entry = entry(
            &source.path().join("file.txt"),
            &target.path().join("target.txt"),
            DeployType::Copy,
        );
        let mut registry = dry_registry(state.path());

        let decision = decide(
            &database,
            target.path(),
            &entry,
            &no_context(),
            &mut registry,
            false,
            true,
        )
        .unwrap();

        assert_eq!(
            decision,
            Decision::Replaced {
                remove: target.path().join("target.txt")
            }
        );
    }
}
