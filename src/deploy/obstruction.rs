//! Asks about and shows obstructions blocking deployment.
//!
//! The deployer decides *what* to remove via `reconcile::decide`; this module
//! owns the interactive prompt and the ViewDiff pager chain. No paths cross
//! this seam: resolution returns a `ResolveAction` and removal stays in
//! `deployer`.

use std::{
    collections::HashMap,
    ffi::OsStr,
    fs,
    io::Write,
    path::Path,
    process::{Child, Command, Stdio},
    result::Result as StdResult,
};

use miette::{Result, WrapErr, miette};
use strum::EnumIter;
use templater::value::Value;
use tui::prompt::{PromptError, PromptOption};

use crate::{
    config::{self, DeployType, DiffCommand, GlobalConfig, PagerCommand},
    internal_error,
    render::RenderRegistry,
    report::diff_sink,
};

/// What the user chose at an obstruction prompt, in prompt order.
#[derive(Debug, Clone, PartialEq, Eq, EnumIter)]
pub enum ObstructionChoice {
    Skip,
    ViewDiff,
    Replace,
    ReplaceAll,
}

impl PromptOption for ObstructionChoice {
    fn hotkey(&self) -> Option<char> {
        match self {
            Self::ReplaceAll => Some('a'),
            _ => None,
        }
    }
}

/// What the interaction resolved to: removal stays in `deployer`, so no path
/// crosses this seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResolveAction {
    Skip,
    Cancel,
    Replace { latch_all: bool },
}

/// Thin prompt seam for ViewDiff-loop tests.
pub trait Prompter {
    fn prompt(
        &self,
        entry: &config::DeploymentEntry,
        obstruction: &Path,
    ) -> StdResult<ObstructionChoice, PromptError>;
}

impl<P: Prompter + ?Sized> Prompter for &P {
    fn prompt(
        &self,
        entry: &config::DeploymentEntry,
        obstruction: &Path,
    ) -> StdResult<ObstructionChoice, PromptError> {
        (**self).prompt(entry, obstruction)
    }
}

/// Thin diff seam for ViewDiff-loop tests.
pub(crate) trait Differ {
    fn show_diff(
        &self,
        entry: &config::DeploymentEntry,
        obstruction: &Path,
        context: &HashMap<String, Value>,
        registry: &mut RenderRegistry,
        global_config: &GlobalConfig,
    ) -> Result<()>;
}

/// Trait seam the deployer depends on.
pub(crate) trait ObstructionResolver {
    fn resolve_obstruction(
        &self,
        entry: &config::DeploymentEntry,
        obstruction: &Path,
        context: &HashMap<String, Value>,
        registry: &mut RenderRegistry,
    ) -> Result<ResolveAction>;
}

/// Real prompter backed by the TUI prompt.
#[derive(Debug, Default)]
pub(crate) struct RealPrompter;

impl Prompter for RealPrompter {
    fn prompt(
        &self,
        entry: &config::DeploymentEntry,
        obstruction: &Path,
    ) -> StdResult<ObstructionChoice, PromptError> {
        prompt_for_obstruction(entry, obstruction)
    }
}

/// Real differ backed by the configured diff command (or the built-in
/// `diff -u`) plus the pager chain.
#[derive(Debug, Default)]
pub(crate) struct RealDiffer;

impl Differ for RealDiffer {
    fn show_diff(
        &self,
        entry: &config::DeploymentEntry,
        obstruction: &Path,
        context: &HashMap<String, Value>,
        registry: &mut RenderRegistry,
        global_config: &GlobalConfig,
    ) -> Result<()> {
        view_diff(entry, obstruction, context, registry, global_config)
    }
}

/// Concrete interaction: loops the ViewDiff prompt internally.
pub(crate) struct Interaction<'a, P = RealPrompter, D = RealDiffer> {
    pub(crate) global_config: &'a GlobalConfig,
    pub(crate) prompter: P,
    pub(crate) differ: D,
}

impl<'a, P, D> Interaction<'a, P, D> {
    pub(crate) fn from_parts(global_config: &'a GlobalConfig, prompter: P, differ: D) -> Self {
        Self {
            global_config,
            prompter,
            differ,
        }
    }
}

impl<P: Prompter, D: Differ> ObstructionResolver for Interaction<'_, P, D> {
    fn resolve_obstruction(
        &self,
        entry: &config::DeploymentEntry,
        obstruction: &Path,
        context: &HashMap<String, Value>,
        registry: &mut RenderRegistry,
    ) -> Result<ResolveAction> {
        loop {
            match self.prompter.prompt(entry, obstruction) {
                Ok(ObstructionChoice::Skip) => return Ok(ResolveAction::Skip),
                Ok(ObstructionChoice::ViewDiff) => {
                    self.differ.show_diff(
                        entry,
                        obstruction,
                        context,
                        registry,
                        self.global_config,
                    )?;
                }
                Ok(ObstructionChoice::Replace) => {
                    return Ok(ResolveAction::Replace { latch_all: false });
                }
                Ok(ObstructionChoice::ReplaceAll) => {
                    return Ok(ResolveAction::Replace { latch_all: true });
                }
                Err(PromptError::Cancelled) => return Ok(ResolveAction::Cancel),
                Err(error) => {
                    return Err(miette!(error).wrap_err("cannot show the obstruction prompt"));
                }
            }
        }
    }
}

pub(crate) fn prompt_for_obstruction(
    entry: &config::DeploymentEntry,
    obstruction: &Path,
) -> StdResult<ObstructionChoice, PromptError> {
    use crossterm::style::Color;

    use crate::platform::prettify_path;

    let question = format!(
        "cannot deploy {} {} because {} {} is already present\nhow would you like to proceed?",
        path_kind(&entry.source_path)?,
        prettify_path(&entry.source_path).display(),
        path_kind(obstruction)?,
        prettify_path(obstruction).display()
    );
    let style = tui::prompt::PromptStyle {
        done_question: Color::Grey,
        ..Default::default()
    };
    let should_show_diff = offers_diff(&entry.source_path, obstruction);
    tui::prompt::SelectPrompt::new()
        .question(question)
        .style(style)
        .filter(move |choice| should_show_diff || *choice != ObstructionChoice::ViewDiff)
        .interact()
}

/// Whether the diff choice makes sense: only for two present regular files.
fn offers_diff(source: &Path, obstruction: &Path) -> bool {
    fs::metadata(source).is_ok_and(|metadata| metadata.is_file())
        && fs::metadata(obstruction).is_ok_and(|metadata| metadata.is_file())
}

fn path_kind(path: &Path) -> std::io::Result<&'static str> {
    let meta = std::fs::symlink_metadata(path)?;
    Ok(if meta.is_dir() {
        "directory"
    } else if meta.is_file() {
        "file"
    } else if meta.is_symlink() {
        "symlink"
    } else {
        "unknown"
    })
}

/// Pure pager precedence: `DOTRIFT_PAGER` > Global config pager > `PAGER`
/// best-effort > `diff_sink`. Empty strings are skipped.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PagerSelection<'a> {
    EnvDotrift(String),
    Config(&'a PagerCommand),
    EnvPager(String),
    Stdout,
}

pub(crate) fn pager_chain<'a>(
    dotrift_pager: Option<String>,
    config: Option<&'a PagerCommand>,
    pager: Option<String>,
) -> PagerSelection<'a> {
    if let Some(command) = dotrift_pager.filter(|value| !value.trim().is_empty()) {
        return PagerSelection::EnvDotrift(command);
    }
    if let Some(pager) = config {
        return PagerSelection::Config(pager);
    }
    if let Some(command) = pager.filter(|value| !value.trim().is_empty()) {
        return PagerSelection::EnvPager(command);
    }
    PagerSelection::Stdout
}

fn nonempty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

pub(crate) fn view_diff(
    entry: &config::DeploymentEntry,
    target: &Path,
    context: &HashMap<String, Value>,
    registry: &mut RenderRegistry,
    global_config: &GlobalConfig,
) -> Result<()> {
    let rendered = if entry.deploy_type == DeployType::Template {
        Some(
            registry
                .ensure_rendered(&entry.source_path, context)?
                .ok_or_else(|| internal_error("template render registry is unavailable"))?
                .path,
        )
    } else {
        None
    };
    let source = rendered
        .as_ref()
        .map_or(entry.source_path.as_path(), |path| path.as_path());
    let diff = global_config.diff();

    std::io::stdout()
        .flush()
        .map_err(|error| miette!(error).wrap_err("cannot flush stdout"))?;

    match pager_chain(
        nonempty_env("DOTRIFT_PAGER"),
        global_config.pager(),
        nonempty_env("PAGER"),
    ) {
        PagerSelection::EnvDotrift(command) => {
            let child = spawn_pager(&command)
                .map_err(|error| miette!(error).wrap_err("cannot run `DOTRIFT_PAGER`"))?;
            diff_through(child, target, &entry.source_path, source, diff)
        }
        PagerSelection::Config(pager) => {
            let child = spawn_config_pager(pager)
                .map_err(|error| miette!(error).wrap_err("cannot run the configured pager"))?;
            diff_through(child, target, &entry.source_path, source, diff)
        }
        PagerSelection::EnvPager(command) => match spawn_pager(&command) {
            Ok(child) => diff_through(child, target, &entry.source_path, source, diff),
            Err(_) => {
                let mut output = diff_sink();
                run_diff_into(target, &entry.source_path, source, diff, &mut output)
            }
        },
        PagerSelection::Stdout => {
            let mut output = diff_sink();
            run_diff_into(target, &entry.source_path, source, diff, &mut output)
        }
    }
}

fn diff_through(
    mut child: Child,
    target: &Path,
    source_label: &Path,
    source: &Path,
    diff: Option<&DiffCommand>,
) -> Result<()> {
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| internal_error("pager stdin is unavailable"))?;
    run_diff_into(target, source_label, source, diff, &mut stdin)?;
    drop(stdin);
    child
        .wait()
        .map_err(|error| miette!(error).wrap_err("cannot wait for the pager"))?;
    Ok(())
}

fn run_diff_into<W: std::io::Write>(
    target: &Path,
    source_label: &Path,
    source: &Path,
    diff: Option<&DiffCommand>,
    dest: &mut W,
) -> Result<()> {
    let mut child = match diff {
        Some(diff) => {
            let args = substitute_diff_args(&diff.args, target, source_label, source);
            Command::new(&diff.command)
                .args(args)
                .stdout(Stdio::piped())
                .spawn()
                .map_err(|error| miette!(error))
                .wrap_err_with(|| {
                    format!("cannot run the configured diff command `{}`", diff.command)
                })?
        }
        None => Command::new("diff")
            .arg("-u")
            .arg("--label")
            .arg(target)
            .arg("--label")
            .arg(source_label)
            .arg(target)
            .arg(source)
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|error| miette!(error).wrap_err("cannot run `diff`"))?,
    };
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| internal_error("diff stdout is unavailable"))?;
    std::io::copy(&mut stdout, dest)
        .map_err(|error| miette!(error).wrap_err("cannot pipe diff output"))?;
    let status = child
        .wait()
        .map_err(|error| miette!(error).wrap_err("cannot wait for the diff command"))?;
    // Exit 0 and 1 are success (1 = differences found, the normal finding);
    // everything else — any other code, or death by a signal — fails the run
    // so a truncated view is never mistaken for "no differences".
    if !matches!(status.code(), Some(0) | Some(1)) {
        return Err(match (diff, status.code()) {
            (Some(diff), Some(code)) => miette!(
                "the configured diff command `{}` exited with status {code}",
                diff.command
            ),
            (Some(diff), None) => miette!(
                "the configured diff command `{}` terminated by a signal",
                diff.command
            ),
            (None, Some(code)) => miette!("`diff` exited with status {code}"),
            (None, None) => miette!("`diff` terminated by a signal"),
        });
    }
    Ok(())
}

/// Substitutes the diff placeholders textually, keeping each element a single
/// argument, and appends the compared paths when args name neither of them
/// (`spec/global-config.md § Placeholder substitution`).
fn substitute_diff_args(
    args: &[String],
    target: &Path,
    source_label: &Path,
    source: &Path,
) -> Vec<String> {
    let target = target.to_string_lossy();
    let source_label = source_label.to_string_lossy();
    let source = source.to_string_lossy();
    let mut names_compared_files = false;
    let substituted = args
        .iter()
        .map(|arg| {
            // One pass over the original text, so a substituted path is never
            // itself rescanned for placeholders.
            let mut result = String::with_capacity(arg.len());
            let mut rest = arg.as_str();
            while let Some(start) = rest.find("${") {
                result.push_str(&rest[..start]);
                let after = &rest[start + 2..];
                let Some(end) = after.find('}') else {
                    // Unreachable for validated args: `validate_placeholders`
                    // rejects an unterminated `${` at config load.
                    result.push_str(&rest[start..]);
                    break;
                };
                let name = &after[..end];
                match name {
                    "target" => {
                        result.push_str(&target);
                        names_compared_files = true;
                    }
                    "source" => {
                        result.push_str(&source);
                        names_compared_files = true;
                    }
                    "target-label" => result.push_str(&target),
                    "source-label" => result.push_str(&source_label),
                    // Unreachable for validated args; pass the raw text
                    // through rather than guess a replacement.
                    _ => result.push_str(&rest[start..start + 2 + end + 1]),
                }
                rest = &after[end + 1..];
            }
            result.push_str(rest);
            result
        })
        .collect::<Vec<_>>();
    if names_compared_files {
        return substituted;
    }
    substituted
        .into_iter()
        .chain([target.into_owned(), source.into_owned()])
        .collect()
}

fn spawn_pager(command: &str) -> std::io::Result<Child> {
    let mut parts = command.split_whitespace();
    let Some(program) = parts.next() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "pager command is empty",
        ));
    };
    spawn_pager_program(program, parts)
}

fn spawn_config_pager(pager: &PagerCommand) -> std::io::Result<Child> {
    spawn_pager_program(&pager.command, &pager.args)
}

fn spawn_pager_program<I, S>(program: &str, args: I) -> std::io::Result<Child>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .spawn()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;

    use tempfile::{TempDir, tempdir};
    use test_case::test_case;

    use super::*;
    use crate::platform::Environment;

    fn test_config_pager() -> PagerCommand {
        PagerCommand {
            command: "config-pager".to_string(),
            args: vec![],
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum ExpectedPager {
        Dotrift(String),
        Config(String, Vec<String>),
        Pager(String),
        Stdout,
    }

    #[test_case(
        Some("dotrift-pager".to_string()),
        None,
        Some("env-pager".to_string()),
        ExpectedPager::Dotrift("dotrift-pager".to_string())
        ; "prefers_dotrift_pager_over_config_and_env"
    )]
    #[test_case(
        None,
        Some(test_config_pager()),
        Some("env-pager".to_string()),
        ExpectedPager::Config("config-pager".to_string(), vec![])
        ; "prefers_config_over_env_pager"
    )]
    #[test_case(
        None,
        None,
        Some("env-pager".to_string()),
        ExpectedPager::Pager("env-pager".to_string())
        ; "uses_env_pager_when_nothing_else_set"
    )]
    #[test_case(
        Some("   ".to_string()),
        Some(test_config_pager()),
        None,
        ExpectedPager::Config("config-pager".to_string(), vec![])
        ; "blank_dotrift_pager_falls_back_to_config"
    )]
    #[test_case(
        Some("   ".to_string()),
        None,
        Some("".to_string()),
        ExpectedPager::Stdout
        ; "skips_empty_strings"
    )]
    #[test_case(
        None,
        None,
        None,
        ExpectedPager::Stdout
        ; "falls_back_to_stdout"
    )]
    fn pager_chain_picks(
        dotrift_pager: Option<String>,
        config: Option<PagerCommand>,
        pager: Option<String>,
        expected: ExpectedPager,
    ) {
        let actual = match pager_chain(dotrift_pager, config.as_ref(), pager) {
            PagerSelection::EnvDotrift(command) => ExpectedPager::Dotrift(command),
            PagerSelection::Config(pager) => {
                ExpectedPager::Config(pager.command.clone(), pager.args.clone())
            }
            PagerSelection::EnvPager(command) => ExpectedPager::Pager(command),
            PagerSelection::Stdout => ExpectedPager::Stdout,
        };
        assert_eq!(actual, expected);
    }

    /// Scripted prompter: pops one choice per prompt; cancels once the queue
    /// is empty.
    struct SeqPrompter {
        choices: RefCell<Vec<ObstructionChoice>>,
        calls: RefCell<usize>,
    }

    impl Prompter for SeqPrompter {
        fn prompt(
            &self,
            _entry: &config::DeploymentEntry,
            _obstruction: &Path,
        ) -> StdResult<ObstructionChoice, PromptError> {
            *self.calls.borrow_mut() += 1;
            self.choices
                .borrow_mut()
                .pop()
                .ok_or(PromptError::Cancelled)
        }
    }

    struct FailingPrompter;

    impl Prompter for FailingPrompter {
        fn prompt(
            &self,
            _: &config::DeploymentEntry,
            _: &Path,
        ) -> StdResult<ObstructionChoice, PromptError> {
            Err(PromptError::Io(std::io::Error::from(
                std::io::ErrorKind::NotFound,
            )))
        }
    }

    struct CountingDiffer {
        calls: RefCell<usize>,
    }

    impl Differ for CountingDiffer {
        fn show_diff(
            &self,
            _entry: &config::DeploymentEntry,
            _obstruction: &Path,
            _context: &HashMap<String, Value>,
            _registry: &mut RenderRegistry,
            _global_config: &GlobalConfig,
        ) -> Result<()> {
            *self.calls.borrow_mut() += 1;
            Ok(())
        }
    }

    fn fixture() -> (TempDir, GlobalConfig, RenderRegistry) {
        let state = tempdir().expect("cannot create temp dir");
        let env = Environment::test_root(state.path());
        let registry = RenderRegistry::acquire(&env);
        (state, GlobalConfig::default(), registry)
    }

    fn resolve_entry(state: &Path) -> config::DeploymentEntry {
        config::DeploymentEntry {
            source_path: state.join("src"),
            target_path: state.join("dst"),
            deploy_type: DeployType::Copy,
            mode: None,
        }
    }

    #[test]
    fn a_viewdiff_then_replace_shows_once_and_replaces() {
        let (state, global_config, mut registry) = fixture();
        let entry = resolve_entry(state.path());
        let interaction = Interaction {
            global_config: &global_config,
            prompter: SeqPrompter {
                choices: RefCell::new(vec![
                    // Popped from the end: ViewDiff first, then Replace.
                    ObstructionChoice::Replace,
                    ObstructionChoice::ViewDiff,
                ]),
                calls: RefCell::new(0),
            },
            differ: CountingDiffer {
                calls: RefCell::new(0),
            },
        };
        let action = interaction
            .resolve_obstruction(&entry, &entry.target_path, &HashMap::new(), &mut registry)
            .expect("resolution succeeds");
        assert_eq!(action, ResolveAction::Replace { latch_all: false });
        assert_eq!(*interaction.prompter.calls.borrow(), 2);
        assert_eq!(*interaction.differ.calls.borrow(), 1);
    }

    #[test]
    fn a_skip_resolves_to_a_skip_action() {
        let (state, global_config, mut registry) = fixture();
        let entry = resolve_entry(state.path());
        let interaction = Interaction {
            global_config: &global_config,
            prompter: SeqPrompter {
                choices: RefCell::new(vec![ObstructionChoice::Skip]),
                calls: RefCell::new(0),
            },
            differ: CountingDiffer {
                calls: RefCell::new(0),
            },
        };
        let action = interaction
            .resolve_obstruction(&entry, &entry.target_path, &HashMap::new(), &mut registry)
            .expect("resolution succeeds");
        assert_eq!(action, ResolveAction::Skip);
        assert_eq!(*interaction.differ.calls.borrow(), 0);
    }

    #[test]
    fn a_replace_all_resolves_to_a_latched_replace() {
        let (state, global_config, mut registry) = fixture();
        let entry = resolve_entry(state.path());
        let interaction = Interaction {
            global_config: &global_config,
            prompter: SeqPrompter {
                choices: RefCell::new(vec![ObstructionChoice::ReplaceAll]),
                calls: RefCell::new(0),
            },
            differ: CountingDiffer {
                calls: RefCell::new(0),
            },
        };
        let action = interaction
            .resolve_obstruction(&entry, &entry.target_path, &HashMap::new(), &mut registry)
            .expect("resolution succeeds");
        assert_eq!(action, ResolveAction::Replace { latch_all: true });
    }

    #[test]
    fn a_cancelled_prompt_parks_as_a_cancel_action() {
        // The queue is empty, so the prompter cancels on the first prompt.
        let (state, global_config, mut registry) = fixture();
        let entry = resolve_entry(state.path());
        let interaction = Interaction {
            global_config: &global_config,
            prompter: SeqPrompter {
                choices: RefCell::new(vec![]),
                calls: RefCell::new(0),
            },
            differ: CountingDiffer {
                calls: RefCell::new(0),
            },
        };
        let action = interaction
            .resolve_obstruction(&entry, &entry.target_path, &HashMap::new(), &mut registry)
            .expect("resolution succeeds");
        assert_eq!(action, ResolveAction::Cancel);
        assert_eq!(*interaction.prompter.calls.borrow(), 1);
    }

    #[test]
    fn a_failing_prompt_is_reported_as_an_error() {
        let (state, global_config, mut registry) = fixture();
        let entry = resolve_entry(state.path());
        let interaction = Interaction {
            global_config: &global_config,
            prompter: FailingPrompter,
            differ: CountingDiffer {
                calls: RefCell::new(0),
            },
        };
        let error = interaction
            .resolve_obstruction(&entry, &entry.target_path, &HashMap::new(), &mut registry)
            .expect_err("the prompt failure must surface");
        assert!(
            error
                .to_string()
                .contains("cannot show the obstruction prompt"),
            "{error}"
        );
    }

    #[test]
    fn a_prompter_behind_a_reference_resolves_through_the_blanket() {
        let (state, global_config, mut registry) = fixture();
        let entry = resolve_entry(state.path());
        let prompter = SeqPrompter {
            choices: RefCell::new(vec![ObstructionChoice::Skip]),
            calls: RefCell::new(0),
        };
        let interaction = Interaction {
            global_config: &global_config,
            prompter: &prompter,
            differ: CountingDiffer {
                calls: RefCell::new(0),
            },
        };
        let action = interaction
            .resolve_obstruction(&entry, &entry.target_path, &HashMap::new(), &mut registry)
            .expect("resolution succeeds");
        assert_eq!(action, ResolveAction::Skip);
    }

    #[test]
    fn the_diff_choice_gates_on_two_regular_files() {
        let state = tempdir().expect("cannot create temp dir");
        let source = state.path().join("file1");
        fs::write(&source, b"content1").expect("cannot write file");
        let obstruction = state.path().join("file2");
        fs::write(&obstruction, b"content2").expect("cannot write file");
        assert!(offers_diff(&source, &obstruction));
    }

    #[test]
    fn the_diff_choice_gates_on_a_directory_obstruction() {
        let state = tempdir().expect("cannot create temp dir");
        let source = state.path().join("file1");
        fs::write(&source, b"content1").expect("cannot write file");
        let obstruction = state.path().join("dir1");
        fs::create_dir(&obstruction).expect("cannot create dir");
        assert!(!offers_diff(&source, &obstruction));
    }

    #[test]
    fn the_diff_choice_gates_on_a_missing_path() {
        let state = tempdir().expect("cannot create temp dir");
        let source = state.path().join("file1");
        fs::write(&source, b"content1").expect("cannot write file");
        assert!(!offers_diff(&source, &state.path().join("missing1")));
        assert!(!offers_diff(&state.path().join("missing1"), &source));
    }
}
