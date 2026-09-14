use std::fs;
use std::path::{Path, PathBuf};

use dotrift::commands::apply::ApplyOptions;
use dotrift::deploy::Prompter;

use super::env::TestEnv;
use super::prompt::Prompt;

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

    /// Core run: explicit options + prompter. All other entry points delegate
    /// here so the interface stays small.
    pub fn try_run_with(
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

    pub fn run_with(&self, options: ApplyOptions, prompter: &dyn Prompter) {
        self.try_run_with(options, prompter).expect("apply failed");
    }

    pub fn run(&self) {
        self.try_run().expect("apply failed");
    }

    pub fn try_run(&self) -> std::result::Result<dotrift::ExitStatus, miette::Report> {
        self.try_run_with(ApplyOptions::default(), &Prompt::never())
    }

    // Compatibility shims over `try_run_with` / `run_with`. New code prefers
    // `run`, `run_with`, `try_run`, `try_run_with`.

    pub fn run_with_options(&self, options: ApplyOptions) {
        self.run_with(options, &Prompt::never());
    }

    pub fn run_with_prompter(&self, prompter: &dyn Prompter) {
        self.run_with(ApplyOptions::default(), prompter);
    }

    pub fn run_with_options_and_prompter(&self, options: ApplyOptions, prompter: &dyn Prompter) {
        self.run_with(options, prompter);
    }

    pub fn try_run_with_options(
        &self,
        options: ApplyOptions,
    ) -> std::result::Result<dotrift::ExitStatus, miette::Report> {
        self.try_run_with(options, &Prompt::never())
    }

    pub fn try_run_with_prompter(
        &self,
        prompter: &dyn Prompter,
    ) -> std::result::Result<dotrift::ExitStatus, miette::Report> {
        self.try_run_with(ApplyOptions::default(), prompter)
    }

    pub fn try_run_with_options_and_prompter(
        &self,
        options: ApplyOptions,
        prompter: &dyn Prompter,
    ) -> std::result::Result<dotrift::ExitStatus, miette::Report> {
        self.try_run_with(options, prompter)
    }
}
