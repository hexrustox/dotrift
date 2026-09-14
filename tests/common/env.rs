use dotrift::platform::Environment;
use dotrift::state::StateDatabase;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use tempfile::TempDir;

pub struct TestEnv {
    root: TempDir,
    env: Environment,
}

impl TestEnv {
    pub fn new() -> Self {
        let root = TempDir::new().expect("cannot create temp dir");
        let env = Environment::test_root(root.path());
        Self { root, env }
    }

    pub fn root(&self) -> &Path {
        self.root.path()
    }

    pub fn env(&self) -> &Environment {
        &self.env
    }

    pub fn path(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.root.path().join(relative)
    }

    pub fn database(&self) -> StateDatabase {
        StateDatabase::open(&self.env).expect("cannot open state database")
    }

    /// Creates and returns `<root>/source`.
    pub fn source_dir(&self) -> PathBuf {
        let source = self.path("source");
        fs::create_dir_all(&source).unwrap();
        source
    }

    /// Creates and returns `<root>/target`.
    pub fn target_dir(&self) -> PathBuf {
        let target = self.path("target");
        fs::create_dir_all(&target).unwrap();
        target
    }

    /// Writes `source/dotrift.toml` (creating `source`).
    pub fn write_config(&self, contents: &str) {
        fs::write(self.source_dir().join("dotrift.toml"), contents).unwrap();
    }

    /// Writes `source/dotrift_data.toml` (creating `source`).
    pub fn write_data_file(&self, contents: &str) {
        fs::write(self.source_dir().join("dotrift_data.toml"), contents).unwrap();
    }

    /// Writes the global config at the path pinned in [`TestEnv::new`]:
    /// `<root>/config-home/dotrift/config.toml`.
    pub fn write_global_config(&self, contents: &str) {
        let path = self.path("config-home/dotrift/config.toml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    /// Sets process env vars for the fixture, restoring previous values on
    /// drop. Folded into the fixture so callers never name the guard type:
    /// `let _guard = env.set_vars([...]);`.
    pub fn set_vars<'a>(
        &self,
        vars: impl IntoIterator<Item = (&'static str, Option<&'a str>)>,
    ) -> impl Drop {
        VarGuard::set(
            vars.into_iter()
                .map(|(name, value)| (name, value.map(str::to_owned))),
        )
    }
}

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct VarGuard {
    previous: Vec<(&'static str, Option<String>)>,
    _guard: MutexGuard<'static, ()>,
}

impl VarGuard {
    fn set(vars: impl IntoIterator<Item = (&'static str, Option<String>)>) -> Self {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        let previous = vars
            .into_iter()
            .map(|(name, value)| {
                let previous = std::env::var(name).ok();
                unsafe {
                    match value {
                        Some(value) => std::env::set_var(name, value),
                        None => std::env::remove_var(name),
                    }
                }
                (name, previous)
            })
            .collect();
        Self { previous, _guard }
    }
}

impl Drop for VarGuard {
    fn drop(&mut self) {
        for (name, previous) in &self.previous {
            unsafe {
                match previous {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}
