#![allow(dead_code)]

use dotrift::commands::apply::ApplyOptions;
use dotrift::deploy::{ObstructionChoice, Prompter};
use dotrift::platform::Environment;
use dotrift::state::StateDatabase;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
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
        Self::set_owned(
            vars.into_iter()
                .map(|(name, value)| (name, value.map(str::to_owned))),
        )
    }

    fn set_owned(vars: impl IntoIterator<Item = (&'static str, Option<String>)>) -> Self {
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
        self.try_run_with_options_and_prompter(options, &PanickingPrompter)
            .expect("apply failed");
    }

    pub fn run_with_prompter(&self, prompter: &dyn Prompter) {
        self.try_run_with_prompter(prompter).expect("apply failed");
    }

    pub fn run_with_options_and_prompter(&self, options: ApplyOptions, prompter: &dyn Prompter) {
        self.try_run_with_options_and_prompter(options, prompter)
            .expect("apply failed");
    }

    pub fn try_run(&self) -> std::result::Result<dotrift::ExitStatus, miette::Report> {
        self.try_run_with_prompter(&PanickingPrompter)
    }

    pub fn try_run_with_options(
        &self,
        options: ApplyOptions,
    ) -> std::result::Result<dotrift::ExitStatus, miette::Report> {
        self.try_run_with_options_and_prompter(options, &PanickingPrompter)
    }

    pub fn try_run_with_prompter(
        &self,
        prompter: &dyn Prompter,
    ) -> std::result::Result<dotrift::ExitStatus, miette::Report> {
        dotrift::commands::apply::run_with_prompter(
            &self.source,
            Some(self.target.clone()),
            self.env.env(),
            false,
            prompter,
        )
    }

    pub fn try_run_with_options_and_prompter(
        &self,
        options: ApplyOptions,
        prompter: &dyn Prompter,
    ) -> std::result::Result<dotrift::ExitStatus, miette::Report> {
        dotrift::commands::apply::run_with_options_and_prompter(
            &self.source,
            Some(self.target.clone()),
            options,
            self.env.env(),
            false,
            prompter,
        )
    }
}

/// Scripted obstruction prompter: answers each prompt from a queue, in order.
/// Panics when the queue is exhausted, so a test that prompts more often than
/// scripted fails loudly instead of silently cancelling.
pub struct QueuePrompter {
    choices: RefCell<VecDeque<ObstructionChoice>>,
    calls: Cell<usize>,
}

impl QueuePrompter {
    pub fn once(choice: ObstructionChoice) -> Self {
        Self::sequence([choice])
    }

    pub fn sequence(choices: impl IntoIterator<Item = ObstructionChoice>) -> Self {
        Self {
            choices: RefCell::new(choices.into_iter().collect()),
            calls: Cell::new(0),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.get()
    }
}

impl Prompter for QueuePrompter {
    fn prompt(
        &self,
        _entry: &dotrift::config::DeploymentEntry,
        _obstruction: &Path,
    ) -> std::result::Result<ObstructionChoice, tui::prompt::PromptError> {
        self.calls.set(self.calls.get() + 1);
        Ok(self
            .choices
            .borrow_mut()
            .pop_front()
            .expect("obstruction prompt choices exhausted by test"))
    }
}

/// Obstruction prompter that cancels the run at the first prompt.
pub struct CancellingPrompter {
    calls: Cell<usize>,
}

impl CancellingPrompter {
    pub fn new() -> Self {
        Self {
            calls: Cell::new(0),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.get()
    }
}

impl Prompter for CancellingPrompter {
    fn prompt(
        &self,
        _entry: &dotrift::config::DeploymentEntry,
        _obstruction: &Path,
    ) -> std::result::Result<ObstructionChoice, tui::prompt::PromptError> {
        self.calls.set(self.calls.get() + 1);
        Err(tui::prompt::PromptError::Cancelled)
    }
}

/// Obstruction prompter that must never fire: any prompt is a test failure.
pub struct PanickingPrompter;

impl Prompter for PanickingPrompter {
    fn prompt(
        &self,
        _entry: &dotrift::config::DeploymentEntry,
        _obstruction: &Path,
    ) -> std::result::Result<ObstructionChoice, tui::prompt::PromptError> {
        panic!("obstruction prompt must not fire in this test");
    }
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
