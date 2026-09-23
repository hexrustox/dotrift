//! The per-run template render registry: rendered template output stored once
//! per apply run, keyed by template hash.

use std::{
    collections::HashMap,
    fs,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use super::template;
use crate::state::{Fingerprint, TemplateHash};

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

/// The rendered output of one template for this run.
#[derive(Debug)]
pub(crate) struct RegistryEntry {
    pub(crate) path: PathBuf,
    pub(crate) digest: Fingerprint,
}

impl RenderRegistry {
    /// Prepares the registry for a run — dry runs included, which render into
    /// it for the identical-obstruction check and leave nothing behind on drop.
    /// A real run empties and re-creates the registry directory, falling back
    /// to no registry when that fails.
    pub(crate) fn acquire(env: &crate::platform::Environment) -> Self {
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

    /// Renders into the registry on first use and copies from it afterwards.
    pub(crate) fn ensure_rendered(
        &mut self,
        source: &Path,
        context: &HashMap<String, templater::value::Value>,
    ) -> miette::Result<Option<RegistryEntry>> {
        let Some(dir) = self.dir.clone() else {
            return Ok(None);
        };
        let template_hash = TemplateHash::of_file(source)?;
        let path = dir.join(format!("{template_hash}.tmpl"));
        if fs::symlink_metadata(&path).is_ok() {
            let digest = match self.memo.get(&template_hash) {
                Some(digest) => digest.clone(),
                None => {
                    let digest = Fingerprint::of_file(&path)?;
                    self.memo.insert(template_hash.clone(), digest.clone());
                    digest
                }
            };
            return Ok(Some(RegistryEntry { path, digest }));
        }
        let file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => file,
            Err(_) => {
                let _ = fs::remove_file(&path);
                return Ok(None);
            }
        };
        let mut writer = crate::state::HashWriter::new(std::io::BufWriter::new(file));
        match template::render_template_into(source, context, &mut writer) {
            Ok(()) => {}
            Err(template::RenderFailure::Sink(_)) => {
                let _ = fs::remove_file(&path);
                return Ok(None);
            }
            Err(template::RenderFailure::Template(report)) => {
                let _ = fs::remove_file(&path);
                return Err(report);
            }
        }
        let digest = writer.into_digest();
        self.memo.insert(template_hash, digest.clone());
        Ok(Some(RegistryEntry { path, digest }))
    }
}

impl Drop for RenderRegistry {
    fn drop(&mut self) {
        if let Some(dir) = &self.dir {
            let _ = fs::remove_dir_all(dir);
        }
    }
}

/// Failure is silent: the registry is an optimization.
fn clear_and_create(dir: &Path) -> bool {
    match fs::remove_dir_all(dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return false,
    }
    fs::create_dir_all(dir).is_ok()
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, os::unix::fs::PermissionsExt};

    use tempfile::tempdir;

    use super::RenderRegistry;
    use crate::platform::Environment;

    fn context() -> HashMap<String, templater::value::Value> {
        HashMap::from([(
            "str".to_string(),
            templater::value::Value::Str("str".into()),
        )])
    }

    #[test]
    fn acquire_falls_back_when_the_registry_dir_cannot_be_created() {
        let root = tempdir().expect("cannot create temp dir");
        fs::write(root.path().join("blocked"), b"").expect("cannot write blocker");
        let env =
            Environment::test_root(root.path()).with_registry_dir(root.path().join("blocked"));

        let template = root.path().join("file1");
        fs::write(&template, b"{{ str }}\n").expect("cannot write template");
        let mut registry = RenderRegistry::acquire(&env);

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
        let mut registry = RenderRegistry::acquire(&env);

        let template = root.path().join("file1");
        fs::write(&template, b"{{ str }}\n").expect("cannot write template");
        let rendered = registry
            .ensure_rendered(&template, &context())
            .expect("cannot render into the registry")
            .expect("the registry must hold the render");

        let bytes = fs::read(&rendered.path).expect("cannot read registry entry");
        assert_eq!(bytes, b"str\n");
        assert_eq!(
            fs::symlink_metadata(&rendered.path)
                .expect("cannot stat registry entry")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(rendered.digest, crate::state::hash_bytes(&bytes));
    }
}
