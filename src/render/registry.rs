//! The per-run template render registry: rendered template output stored once
//! per apply run, keyed by template hash.

use std::{
    collections::HashMap,
    fs,
    io::{self, BufWriter},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use miette::Result;
use templater::value::Value;

use super::template;
use crate::{
    platform::Environment,
    state::{Fingerprint, HashWriter, TemplateHash},
};

/// The rendered output of one template for this run.
#[derive(Debug)]
pub(crate) struct Rendered {
    /// The registry entry holding the rendered bytes.
    pub(crate) path: PathBuf,
    /// The fingerprint of the rendered bytes.
    pub(crate) digest: Fingerprint,
}

/// A per-run, content-addressed store of rendered template output.
///
/// Valid only for the run that filled it (ADR-0017): the registry is emptied
/// when the run begins and emptied again on drop, so a run can never observe
/// another run's renders. Registry infrastructure failures disable the
/// registry for the run — [`RenderRegistry::ensure_rendered`] then reports
/// `Ok(None)`, so template deploys render directly into their targets while
/// template view diffs fail — while template-engine errors always propagate.
pub(crate) struct RenderRegistry {
    dir: Option<PathBuf>,
    memo: HashMap<TemplateHash, Fingerprint>,
}

impl RenderRegistry {
    /// Prepares the registry for a run. A dry run constructs nothing;
    /// a real run empties and re-creates the registry directory, falling
    /// back to no registry when that fails.
    pub(crate) fn acquire(env: &Environment, dry_run: bool) -> Self {
        if dry_run {
            return Self {
                dir: None,
                memo: HashMap::new(),
            };
        }
        let dir = env.registry_dir();
        if clear_and_create(&dir) {
            Self {
                dir: Some(dir),
                memo: HashMap::new(),
            }
        } else {
            Self {
                dir: None,
                memo: HashMap::new(),
            }
        }
    }

    /// Returns the registry entry for `source`'s render, rendering into the
    /// registry on first use and copying from it afterwards.
    pub(crate) fn ensure_rendered(
        &mut self,
        source: &Path,
        context: &HashMap<String, Value>,
    ) -> Result<Option<Rendered>> {
        let Some(dir) = self.dir.clone() else {
            return Ok(None);
        };
        let template_hash = TemplateHash::of_file(source)?;
        let entry = dir.join(format!("{template_hash}.tmpl"));
        if fs::symlink_metadata(&entry).is_ok() {
            let digest = match self.memo.get(&template_hash) {
                Some(digest) => digest.clone(),
                None => {
                    let digest = Fingerprint::of_file(&entry)?;
                    self.memo.insert(template_hash.clone(), digest.clone());
                    digest
                }
            };
            return Ok(Some(Rendered {
                path: entry,
                digest,
            }));
        }
        let file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&entry)
        {
            Ok(file) => file,
            Err(_) => {
                let _ = fs::remove_file(&entry);
                return Ok(None);
            }
        };
        let mut writer = HashWriter::new(BufWriter::new(file));
        match template::render_template_into(source, context, &mut writer) {
            Ok(()) => {}
            Err(template::RenderFailure::Sink(_)) => {
                let _ = fs::remove_file(&entry);
                return Ok(None);
            }
            Err(template::RenderFailure::Template(report)) => {
                let _ = fs::remove_file(&entry);
                return Err(report);
            }
        }
        let digest = writer.into_digest();
        self.memo.insert(template_hash, digest.clone());
        Ok(Some(Rendered {
            path: entry,
            digest,
        }))
    }
}

impl Drop for RenderRegistry {
    fn drop(&mut self) {
        if let Some(dir) = &self.dir {
            let _ = fs::remove_dir_all(dir);
        }
    }
}

/// Empties and re-creates the registry directory, reporting whether it is
/// usable. Failure is silent: the registry is an optimization.
fn clear_and_create(dir: &Path) -> bool {
    match fs::remove_dir_all(dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return false,
    }
    fs::create_dir_all(dir).is_ok()
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, os::unix::fs::PermissionsExt};

    use tempfile::tempdir;

    use super::RenderRegistry;
    use crate::{platform::Environment, state::hash_bytes};
    use templater::value::Value;

    fn context() -> HashMap<String, Value> {
        HashMap::from([("greeting".to_string(), Value::Str("hello".into()))])
    }

    #[test]
    fn acquire_falls_back_when_the_registry_dir_cannot_be_created() {
        let root = tempdir().expect("cannot create temp dir");
        fs::write(root.path().join("blocked"), b"").expect("cannot write blocker");
        let env =
            Environment::test_root(root.path()).with_registry_dir(root.path().join("blocked"));

        let template = root.path().join("greeting.txt");
        fs::write(&template, b"{{ greeting }}\n").expect("cannot write template");
        let mut registry = RenderRegistry::acquire(&env, false);

        assert!(
            registry
                .ensure_rendered(&template, &context())
                .expect("infrastructure failure must stay silent")
                .is_none()
        );
    }

    #[test]
    fn registry_entry_holds_the_render_with_owner_only_permissions() {
        let root = tempdir().expect("cannot create temp dir");
        let env = Environment::test_root(root.path());
        let mut registry = RenderRegistry::acquire(&env, false);

        let template = root.path().join("greeting.txt");
        fs::write(&template, b"{{ greeting }}\n").expect("cannot write template");
        let rendered = registry
            .ensure_rendered(&template, &context())
            .expect("cannot render into the registry")
            .expect("the registry must hold the render");

        let bytes = fs::read(&rendered.path).expect("cannot read registry entry");
        assert_eq!(bytes, b"hello\n");
        assert_eq!(
            fs::symlink_metadata(&rendered.path)
                .expect("cannot stat registry entry")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(rendered.digest, hash_bytes(&bytes));
    }
}
