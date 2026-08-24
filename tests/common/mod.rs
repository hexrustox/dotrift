#![allow(dead_code)]

use dotrift::commands::apply::ApplyOptions;
use dotrift::state::{StateDatabase, test_hooks::TEST_STATE_ROOT};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use tempfile::TempDir;

pub struct TestEnv {
    root: TempDir,
}

impl TestEnv {
    pub fn new() -> Self {
        let root = TempDir::new().expect("cannot create temp dir");
        TEST_STATE_ROOT.with_borrow_mut(|r| *r = Some(root.path().join("state")));
        Self { root }
    }

    pub fn root(&self) -> &Path {
        self.root.path()
    }

    pub fn path(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.root.path().join(relative)
    }

    pub fn database(&self) -> StateDatabase {
        StateDatabase::open().expect("cannot open state database")
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
}

/// `insta` settings that filter this env's temp root out of snapshot output.
///
/// `assert_snapshot!` must still be invoked from the test file (not from a
/// shared helper) so insta derives the correct snapshot name and `tests/`
/// directory from the calling module.
pub fn snapshot_settings(env: &TestEnv) -> insta::Settings {
    let mut settings = insta::Settings::new();
    settings.add_filter(env.root().to_str().unwrap(), "<root>");
    settings
}

/// The current test's generated name, stable under `#[test_case]` labels.
pub fn test_name() -> String {
    std::thread::current().name().unwrap().replace(":", "_")
}

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Sets a set of environment variables and restores their previous values on
/// drop.
pub struct EnvVarGuard {
    previous: Vec<(&'static str, Option<String>)>,
    _guard: MutexGuard<'static, ()>,
}

impl EnvVarGuard {
    pub fn set<'a>(vars: impl IntoIterator<Item = (&'static str, Option<&'a str>)>) -> Self {
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

impl Drop for EnvVarGuard {
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

/// Reconciles a source tree against a target tree with `dotrift.toml` written
/// from a setup closure. Exposes the run/twice patterns shared by the apply
/// integration tests.
pub struct ApplyScenario {
    pub env: TestEnv,
    pub source: PathBuf,
    pub target: PathBuf,
    config: &'static str,
}

impl ApplyScenario {
    pub fn new(setup: impl Fn(&Path, &Path) -> &'static str) -> Self {
        let env = TestEnv::new();
        let source = env.source_dir();
        let target = env.target_dir();
        let config = setup(&source, &target);
        fs::write(source.join("dotrift.toml"), config).unwrap();
        Self {
            env,
            source,
            target,
            config,
        }
    }

    pub fn write_config(&self, contents: &str) {
        fs::write(self.source.join("dotrift.toml"), contents).unwrap();
    }

    /// Re-writes the config from a modify closure, falling back to the
    /// original setup config when the closure returns `None`.
    pub fn rewrite(&self, modify: impl Fn(&Path, &Path) -> Option<&'static str>) {
        self.write_config(modify(&self.source, &self.target).unwrap_or(self.config));
    }

    pub fn run(&self) {
        self.try_run().expect("apply failed");
    }

    pub fn run_with_options(&self, options: ApplyOptions) {
        self.try_run_with_options(options).expect("apply failed");
    }

    pub fn try_run(&self) -> std::result::Result<dotrift::ExitStatus, miette::Report> {
        dotrift::commands::apply::run(&self.source, Some(self.target.clone()))
    }

    pub fn try_run_with_options(
        &self,
        options: ApplyOptions,
    ) -> std::result::Result<dotrift::ExitStatus, miette::Report> {
        dotrift::commands::apply::run_with_options(&self.source, Some(self.target.clone()), options)
    }
}

/// Number of obstruction prompts fired by the current test.
pub fn prompt_count() -> usize {
    dotrift::commands::apply::test_hooks::PROMPT_COUNT.with(|count| *count.borrow())
}

/// Asserts that some cause in `error`'s chain contains `needle`.
pub fn assert_error_chain(error: &miette::Report, needle: &str) {
    assert!(
        error
            .chain()
            .any(|cause| cause.to_string().contains(needle)),
        "expected an error containing `{needle}` but got: {error:?}"
    );
}
