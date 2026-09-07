use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fs,
    io::{self, BufWriter, Write},
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crossterm::style::Color;
use miette::{Result, WrapErr, miette};
use strum::EnumIter;
use templater::value::Value;
use tui::{
    apply_color,
    prompt::{PromptError, PromptOption},
};

use crate::{
    ExitStatus, color_enabled,
    config::{self, DeployType},
    global_config::{GlobalConfig, PagerCommand},
    hash, managed, prettify_path, println_capture,
    reconcile::{Decision, decide},
    render_registry::RenderRegistry,
    state::{Kind, StateDatabase, StateLock, StateRecord},
    template,
};

/// Reconciles the desired deployment with the target directory.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApplyOptions {
    pub clean_up: bool,
    pub prune_empty_dirs: bool,
    pub dry_run: bool,
    pub quiet: bool,
    pub verbose: bool,
}

pub fn run(source: &Path, target_override: Option<PathBuf>) -> Result<ExitStatus> {
    run_with_options(source, target_override, ApplyOptions::default())
}

pub fn run_with_options(
    source: &Path,
    target_override: Option<PathBuf>,
    options: ApplyOptions,
) -> Result<ExitStatus> {
    let _lock = StateLock::acquire()?;
    let global_config = GlobalConfig::load()?;
    let mut registry = RenderRegistry::acquire(options.dry_run);
    let deployment = config::read(source, target_override)?;
    let target = &deployment.target_directory;

    if fs::symlink_metadata(target).is_ok()
        && !fs::metadata(target).is_ok_and(|metadata| metadata.is_dir())
    {
        return Err(miette!(
            "target directory `{}` is not a directory",
            target.display()
        ));
    }
    if !deployment.entries.is_empty() && fs::symlink_metadata(target).is_err() && !options.dry_run {
        fs::create_dir_all(target)
            .map_err(|error| miette!(error).wrap_err("cannot create target directory"))?;
    }

    let database = StateDatabase::open()?;
    let mut entries = deployment.entries.clone();
    entries.sort_by(|left, right| left.target_path.cmp(&right.target_path));
    let mut replace_all = false;
    let mut skipped = 0;
    let mut deployed = 0;
    let mut replaced = 0;
    for entry in entries {
        if options.dry_run {
            report_dry_run_entry(
                &database,
                target,
                &entry,
                &deployment.variable_context,
                &mut registry,
                &global_config,
            )?;
            continue;
        }
        match deploy_entry(
            &database,
            target,
            &entry,
            &deployment.variable_context,
            &mut registry,
            &mut replace_all,
            &global_config,
        )? {
            EntryResult::Deployed => {
                deployed += 1;
                if options.verbose {
                    println_capture!(
                        "{} {}",
                        apply_color("deployed", Color::Green, color_enabled!()),
                        prettify_path(&entry.target_path).display()
                    );
                }
            }
            EntryResult::Replaced => {
                replaced += 1;
                if options.verbose {
                    println_capture!(
                        "{} {}",
                        apply_color("replaced", Color::Cyan, color_enabled!()),
                        prettify_path(&entry.target_path).display()
                    );
                }
            }
            EntryResult::Skipped => {
                skipped += 1;
                if options.verbose {
                    println_capture!(
                        "{} {}",
                        apply_color("skipped", Color::DarkGrey, color_enabled!()),
                        prettify_path(&entry.target_path).display()
                    );
                }
            }
            EntryResult::Cancelled => return Ok(ExitStatus::Cancelled),
        }
    }
    if options.dry_run {
        if options.clean_up {
            let desired = deployment
                .entries
                .iter()
                .map(|entry| entry.target_path.clone())
                .collect();
            let _ = cleanup(&database, target, &desired, options)?;
        }
        return Ok(ExitStatus::Success);
    }
    let mut removed = 0;
    let mut pruned = 0;
    if options.clean_up && skipped == 0 {
        let desired = deployment
            .entries
            .iter()
            .map(|entry| entry.target_path.clone())
            .collect();
        (removed, pruned) = cleanup(&database, target, &desired, options)?;
    }
    if !options.quiet {
        if options.clean_up {
            println_capture!(
                "deployed {deployed}, replaced {replaced}, skipped {skipped}, removed {removed}, pruned {pruned}"
            );
        } else {
            println_capture!("deployed {deployed}, replaced {replaced}, skipped {skipped}");
        }
    }
    if skipped > 0 {
        return Ok(ExitStatus::Skipped);
    }
    Ok(ExitStatus::Success)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryResult {
    Deployed,
    Replaced,
    Skipped,
    Cancelled,
}

fn deploy_entry(
    database: &StateDatabase,
    target_root: &Path,
    entry: &config::DeploymentEntry,
    context: &HashMap<String, Value>,
    registry: &mut RenderRegistry,
    replace_all: &mut bool,
    global_config: &GlobalConfig,
) -> Result<EntryResult> {
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
        database,
        target_root,
        entry,
        context,
        registry,
        *replace_all,
        global_config.replace_identical(),
    )? {
        Decision::Deployed => {}
        Decision::Replaced { remove } => {
            remove_path(database, &remove)?;
            replaced = true;
        }
        Decision::Prompt(obstruction) => loop {
            match prompt_for_obstruction(entry, &obstruction) {
                Ok(ObstructionChoice::Skip) => return Ok(EntryResult::Skipped),
                Ok(ObstructionChoice::ViewDiff) => {
                    view_diff(entry, &obstruction, context, registry, global_config)?
                }
                Ok(ObstructionChoice::Replace) => {
                    remove_path(database, &obstruction)?;
                    replaced = true;
                    break;
                }
                Ok(ObstructionChoice::ReplaceAll) => {
                    *replace_all = true;
                    remove_path(database, &obstruction)?;
                    replaced = true;
                    break;
                }
                Err(PromptError::Cancelled) => return Ok(EntryResult::Cancelled),
                Err(error) => {
                    return Err(miette!(error).wrap_err("cannot display obstruction prompt"));
                }
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
            let content_hash = write_deployed_file(entry, context, registry)?;
            StateRecord {
                target_path: entry.target_path.clone(),
                source_path: entry.source_path.clone(),
                kind: Kind::File,
                content_hash: Some(content_hash),
            }
        }
    };
    database.put(&record)?;
    if let Some(mode) = entry.mode {
        fs::set_permissions(&entry.target_path, fs::Permissions::from_mode(mode.into()))
            .map_err(|error| miette!(error))
            .wrap_err("cannot apply target mode")?;
    }
    Ok(if replaced {
        EntryResult::Replaced
    } else {
        EntryResult::Deployed
    })
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
) -> Result<String> {
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
    let mut writer = hash::HashWriter::new(BufWriter::new(file));
    let outcome = match entry.deploy_type {
        DeployType::Template => {
            template::render_template_to(&entry.source_path, context, &mut writer)
        }
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

fn report_dry_run_entry(
    database: &StateDatabase,
    target_root: &Path,
    entry: &config::DeploymentEntry,
    context: &HashMap<String, Value>,
    registry: &mut RenderRegistry,
    global_config: &GlobalConfig,
) -> Result<()> {
    let (action, color) = match decide(
        database,
        target_root,
        entry,
        context,
        registry,
        false,
        global_config.replace_identical(),
    )? {
        Decision::Deployed => ("deployed", Color::Green),
        Decision::Replaced { .. } => ("replaced", Color::Cyan),
        Decision::Prompt(_) => ("obstruction", Color::Yellow),
    };
    let deploy_type = match entry.deploy_type {
        DeployType::Symlink => "symlink",
        DeployType::Copy => "copy",
        DeployType::Template => "template",
    };
    let suffix = match entry.mode {
        Some(mode) => format!("[{deploy_type} {:03o}]", u32::from(mode)),
        None => format!("[{deploy_type}]"),
    };
    println_capture!(
        "{} {} {suffix}",
        apply_color(action, color, color_enabled!()),
        prettify_path(&entry.target_path).display()
    );
    Ok(())
}

fn cleanup(
    database: &StateDatabase,
    target_root: &Path,
    desired: &HashSet<PathBuf>,
    options: ApplyOptions,
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
        if !managed::is_managed(&record)? {
            if !dry_run {
                database.remove(path)?;
            }
            continue;
        }
        if dry_run {
            planned_removals.insert(path.clone());
            println_capture!(
                "{} {}",
                apply_color("removed", Color::Red, color_enabled!()),
                prettify_path(path).display()
            );
            continue;
        }
        remove_path(database, path)?;
        removed += 1;
        if options.verbose {
            println_capture!(
                "{} {}",
                apply_color("removed", Color::Red, color_enabled!()),
                prettify_path(path).display()
            );
        }
        if options.prune_empty_dirs {
            pruned += prune_parents(target_root, path, options.verbose)?;
        }
    }
    if dry_run && options.prune_empty_dirs {
        report_dry_run_pruning(target_root, &planned_removals)?;
    }
    Ok((removed, pruned))
}

fn report_dry_run_pruning(target_root: &Path, removals: &HashSet<PathBuf>) -> Result<()> {
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
            println_capture!(
                "{} {}",
                apply_color("pruned", Color::Magenta, color_enabled!()),
                prettify_path(&directory).display()
            );
            planned.insert(directory.clone());
            current = directory.parent().map(Path::to_path_buf);
        }
    }
    Ok(())
}

fn would_be_empty(path: &Path, removals: &HashSet<std::path::PathBuf>) -> Result<bool> {
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

fn prune_parents(target_root: &Path, removed_path: &Path, verbose: bool) -> Result<usize> {
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
        if verbose {
            println_capture!(
                "{} {}",
                apply_color("pruned", Color::Magenta, color_enabled!()),
                prettify_path(parent).display()
            );
        }
        current = parent.parent();
    }
    Ok(count)
}

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

#[cfg(any(test, feature = "testing"))]
pub mod test_hooks {
    use std::cell::RefCell;

    use super::ObstructionChoice;

    pub enum PromptChoices {
        Single(Option<ObstructionChoice>),
        Sequence(Vec<ObstructionChoice>),
    }

    thread_local! {
        pub static PROMPT_CHOICE: RefCell<PromptChoices> = const { RefCell::new(PromptChoices::Single(None)) };
        pub static PROMPT_COUNT: RefCell<usize> = const { RefCell::new(0) };
    }

    pub fn set_prompt_choice(choice: ObstructionChoice) {
        PROMPT_CHOICE.with(|current| *current.borrow_mut() = PromptChoices::Single(Some(choice)));
    }

    pub fn set_prompt_choices(choices: impl IntoIterator<Item = ObstructionChoice>) {
        let mut choices: Vec<_> = choices.into_iter().collect();
        choices.reverse();
        PROMPT_CHOICE.with(|current| *current.borrow_mut() = PromptChoices::Sequence(choices));
    }
}

fn prompt_for_obstruction(
    #[allow(unused_variables)] entry: &config::DeploymentEntry,
    #[allow(unused_variables)] obstruction: &Path,
) -> std::result::Result<ObstructionChoice, PromptError> {
    #[cfg(any(test, feature = "testing"))]
    {
        use test_hooks::{PROMPT_CHOICE, PROMPT_COUNT, PromptChoices};

        PROMPT_COUNT.with_borrow_mut(|count| *count += 1);
        PROMPT_CHOICE.with(|current| match &mut *current.borrow_mut() {
            PromptChoices::Single(None) => Err(PromptError::Cancelled),
            PromptChoices::Single(Some(choice)) => Ok(choice.clone()),
            PromptChoices::Sequence(choices) => Ok(choices
                .pop()
                .expect("obstruction prompt choices exhausted by test")),
        })
    }

    #[cfg(not(any(test, feature = "testing")))]
    {
        use std::{fs, path::Path};

        use crossterm::style::Color;

        fn path_kind(path: &Path) -> std::io::Result<&'static str> {
            let meta = fs::symlink_metadata(path)?;
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

        let question = format!(
            "Cannot deploy {} {} because {} {} is already present.\nHow would you like to proceed?",
            path_kind(&entry.source_path)?,
            prettify_path(&entry.source_path).display(),
            path_kind(obstruction)?,
            prettify_path(obstruction).display()
        );
        let style = tui::prompt::PromptStyle {
            done_question: Color::Grey,
            ..Default::default()
        };
        let should_show_diff = fs::metadata(&entry.source_path)
            .is_ok_and(|metadata| metadata.is_file())
            && fs::metadata(obstruction).is_ok_and(|metadata| metadata.is_file());
        tui::prompt::SelectPrompt::new()
            .question(question)
            .style(style)
            .filter(move |choice| should_show_diff || *choice != ObstructionChoice::ViewDiff)
            .interact()
    }
}

fn view_diff(
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
                .ok_or_else(|| miette!("template render registry is unavailable"))?
                .path,
        )
    } else {
        None
    };
    let source = rendered
        .as_ref()
        .map_or(entry.source_path.as_path(), |path| path.as_path());

    std::io::stdout().flush().map_err(|error| miette!(error))?;

    let env_pager = |name: &str| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
    };

    if let Some(command) = env_pager("DOTRIFT_PAGER") {
        let child = spawn_pager(&command)
            .map_err(|error| miette!(error).wrap_err("cannot run DOTRIFT_PAGER"))?;
        return diff_through(child, target, &entry.source_path, source);
    }
    if let Some(pager) = global_config.pager() {
        let child = spawn_config_pager(pager)
            .map_err(|error| miette!(error).wrap_err("cannot run the configured pager"))?;
        return diff_through(child, target, &entry.source_path, source);
    }
    if let Some(command) = env_pager("PAGER")
        && let Ok(child) = spawn_pager(&command)
    {
        return diff_through(child, target, &entry.source_path, source);
    }
    let mut output = diff_output();
    run_diff_into(target, &entry.source_path, source, &mut output)
}

fn diff_through(
    mut child: std::process::Child,
    target: &Path,
    source_label: &Path,
    source: &Path,
) -> Result<()> {
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| miette!("pager stdin is unavailable"))?;
    run_diff_into(target, source_label, source, &mut stdin)?;
    drop(stdin);
    child.wait().map_err(|error| miette!(error))?;
    Ok(())
}

fn run_diff_into<W: Write>(
    target: &Path,
    source_label: &Path,
    source: &Path,
    dest: &mut W,
) -> Result<()> {
    let mut child = Command::new("diff")
        .arg("-u")
        .arg("--label")
        .arg(target)
        .arg("--label")
        .arg(source_label)
        .arg(target)
        .arg(source)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| miette!(error).wrap_err("cannot run diff"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| miette!("diff stdout is unavailable"))?;
    std::io::copy(&mut stdout, dest).map_err(|error| miette!(error))?;
    if child.wait().map_err(|error| miette!(error))?.code() == Some(2) {
        return Err(miette!("diff exited with an error"));
    }
    Ok(())
}

#[cfg(not(feature = "testing"))]
fn diff_output() -> std::io::Stdout {
    std::io::stdout()
}

#[cfg(feature = "testing")]
fn diff_output() -> crate::capture::CaptureWriter {
    crate::capture::CaptureWriter
}

fn spawn_pager(command: &str) -> std::io::Result<std::process::Child> {
    let mut parts = command.split_whitespace();
    let Some(program) = parts.next() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "pager command is empty",
        ));
    };
    spawn_pager_program(program, parts)
}

fn spawn_config_pager(pager: &PagerCommand) -> std::io::Result<std::process::Child> {
    spawn_pager_program(&pager.command, &pager.args)
}

fn spawn_pager_program<I, S>(program: &str, args: I) -> std::io::Result<std::process::Child>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .spawn()
}

fn remove_path(database: &StateDatabase, path: &Path) -> Result<()> {
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
    use super::*;
    use tempfile::tempdir;
    use test_case::test_case;

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
        prune_parents(dir.path(), &setup(dir.path()), false)
            .unwrap_or_else(|error| panic!("{error}"));
        assert(dir.path());
    }
}
