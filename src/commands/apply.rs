// TODO optimize template rendering & hash calculating once only for each file
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use miette::{Result, WrapErr, miette};
use strum::EnumIter;
use templater::value::Value;
use tui::prompt::{PromptError, PromptOption};

use crate::{
    ExitStatus,
    config::{self, DeployType},
    hash, managed, println_capture,
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
    let deployment = config::read(source, target_override)?;
    let target = &deployment.target_directory;

    if fs::metadata(target).is_ok_and(|metadata| !metadata.is_dir()) {
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
            report_dry_run_entry(&database, target, &entry)?;
            continue;
        }
        match deploy_entry(
            &database,
            target,
            &entry,
            &deployment.variable_context,
            &mut replace_all,
        )? {
            EntryResult::Deployed => {
                deployed += 1;
                if options.verbose {
                    println_capture!("deployed {}", entry.target_path.display());
                }
            }
            EntryResult::Replaced => {
                replaced += 1;
                if options.verbose {
                    println_capture!("replaced {}", entry.target_path.display());
                }
            }
            EntryResult::Skipped => {
                skipped += 1;
                if options.verbose {
                    println_capture!("skipped {}", entry.target_path.display());
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
    replace_all: &mut bool,
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

    let obstruction = parent_obstruction(target_root, &entry.target_path)?;
    let existed = if obstruction.is_some() {
        false
    } else {
        match fs::symlink_metadata(&entry.target_path) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(miette!(error).wrap_err(format!(
                    "cannot inspect target `{}`",
                    entry.target_path.display()
                )));
            }
        }
    };
    let mut replaced = false;
    if let Some(obstruction) = obstruction {
        if !*replace_all {
            loop {
                match prompt_for_obstruction(entry, &obstruction) {
                    Ok(ObstructionChoice::Skip) => return Ok(EntryResult::Skipped),
                    Ok(ObstructionChoice::ViewDiff) => view_diff(entry, &obstruction, context)?,
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
            }
        } else {
            remove_path(database, &obstruction)?;
            replaced = true;
        }
    } else if existed {
        let old_record = database.record(&entry.target_path)?;
        let managed = old_record
            .as_ref()
            .map(managed::is_managed)
            .transpose()?
            .unwrap_or(false);
        if managed {
            remove_path(database, &entry.target_path)?;
            replaced = true;
        } else if !*replace_all {
            loop {
                match prompt_for_obstruction(entry, &entry.target_path) {
                    Ok(ObstructionChoice::Skip) => return Ok(EntryResult::Skipped),
                    Ok(ObstructionChoice::ViewDiff) => {
                        view_diff(entry, &entry.target_path, context)?
                    }
                    Ok(ObstructionChoice::Replace) => {
                        remove_path(database, &entry.target_path)?;
                        replaced = true;
                        break;
                    }
                    Ok(ObstructionChoice::ReplaceAll) => {
                        *replace_all = true;
                        remove_path(database, &entry.target_path)?;
                        replaced = true;
                        break;
                    }
                    Err(PromptError::Cancelled) => return Ok(EntryResult::Cancelled),
                    Err(error) => {
                        return Err(miette!(error).wrap_err("cannot display obstruction prompt"));
                    }
                }
            }
        } else {
            remove_path(database, &entry.target_path)?;
            replaced = true;
        }
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
            let bytes = if entry.deploy_type == DeployType::Template {
                template::render_template(&entry.source_path, context)?
            } else {
                fs::read(&entry.source_path)
                    .map_err(|error| miette!(error))
                    .wrap_err("cannot read copy source")?
            };
            fs::write(&entry.target_path, &bytes)
                .map_err(|error| miette!(error))
                .wrap_err("cannot write target file")?;
            StateRecord {
                target_path: entry.target_path.clone(),
                source_path: entry.source_path.clone(),
                kind: Kind::File,
                content_hash: Some(hash::hash_bytes(&bytes)),
            }
        }
    };
    database.put(&record)?;
    if let Some(mode) = entry.mode {
        fs::set_permissions(&entry.target_path, fs::Permissions::from_mode(mode.into()))
            .map_err(|error| miette!(error))
            .wrap_err("cannot apply target mode")?;
    }
    Ok(if existed || replaced {
        EntryResult::Replaced
    } else {
        EntryResult::Deployed
    })
}

fn report_dry_run_entry(
    database: &StateDatabase,
    target_root: &Path,
    entry: &config::DeploymentEntry,
) -> Result<()> {
    let obstruction = parent_obstruction(target_root, &entry.target_path)?;
    let target_exists = fs::symlink_metadata(&entry.target_path).is_ok();
    let action = if obstruction.is_some()
        || (target_exists && !is_target_managed(database, &entry.target_path)?)
    {
        "obstruction"
    } else if target_exists {
        "replaced"
    } else {
        "deployed"
    };
    println_capture!("{action} {}", entry.target_path.display());
    Ok(())
}

fn is_target_managed(database: &StateDatabase, path: &Path) -> Result<bool> {
    match database.record(path)? {
        Some(record) => managed::is_managed(&record),
        None => Ok(false),
    }
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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
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
            println_capture!("removed {}", path.display());
            continue;
        }
        remove_path(database, path)?;
        removed += 1;
        if options.verbose {
            println_capture!("removed {}", path.display());
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
            println_capture!("pruned {}", directory.display());
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
            println_capture!("pruned {}", parent.display());
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
mod test_hooks {
    use std::cell::RefCell;

    use super::ObstructionChoice;

    pub enum PromptChoices {
        Single(Option<ObstructionChoice>),
        Sequence(Vec<ObstructionChoice>),
    }

    thread_local! {
        pub static PROMPT_CHOICE: RefCell<PromptChoices> =
            const { RefCell::new(PromptChoices::Single(None)) };
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

#[cfg(any(test, feature = "testing"))]
pub use test_hooks::{PROMPT_CHOICE, PROMPT_COUNT, set_prompt_choice, set_prompt_choices};

#[cfg_attr(any(test, feature = "testing"), allow(unused_variables))]
fn prompt_for_obstruction(
    entry: &config::DeploymentEntry,
    obstruction: &Path,
) -> std::result::Result<ObstructionChoice, PromptError> {
    #[cfg(any(test, feature = "testing"))]
    {
        PROMPT_COUNT.with_borrow_mut(|count| *count += 1);
        Ok(
            PROMPT_CHOICE.with(|current| match &mut *current.borrow_mut() {
                test_hooks::PromptChoices::Single(choice) => choice
                    .as_ref()
                    .expect("obstruction prompt reached without a test choice set")
                    .clone(),
                test_hooks::PromptChoices::Sequence(choices) => choices
                    .pop()
                    .expect("obstruction prompt choices exhausted by test"),
            }),
        )
    }

    #[cfg(not(any(test, feature = "testing")))]
    {
        use crossterm::style::Color;

        let question = format!(
            "Cannot deploy {} {} because {} {} is already present.\nHow would you like to proceed?",
            path_kind(&entry.source_path)?,
            entry.source_path.display(),
            path_kind(obstruction)?,
            obstruction.display()
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

#[cfg(not(any(test, feature = "testing")))]
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

fn view_diff(
    entry: &config::DeploymentEntry,
    target: &Path,
    context: &HashMap<String, Value>,
) -> Result<()> {
    let rendered = if entry.deploy_type == DeployType::Template {
        let rendered = template::render_template(&entry.source_path, context)?;
        Some(render_to_temp(&rendered)?)
    } else {
        None
    };
    let source = rendered
        .as_ref()
        .map_or(entry.source_path.as_path(), |path| path.as_path());

    std::io::stdout().flush().map_err(|error| miette!(error))?;

    enum PagerResolution<'a> {
        DotriftPager(&'a str),
        Pager(&'a str),
        Stdout,
    }

    let dotrift_pager = std::env::var("DOTRIFT_PAGER").ok();
    let pager = std::env::var("PAGER").ok();
    let resolution = match dotrift_pager.as_deref() {
        Some(command) if !command.trim().is_empty() => PagerResolution::DotriftPager(command),
        _ => match pager.as_deref() {
            Some(command) if !command.trim().is_empty() => PagerResolution::Pager(command),
            _ => PagerResolution::Stdout,
        },
    };
    match resolution {
        PagerResolution::DotriftPager(command) => {
            let mut child = spawn_pager(command)
                .map_err(|error| miette!(error).wrap_err("cannot run DOTRIFT_PAGER"))?;
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| miette!("pager stdin is unavailable"))?;
            run_diff_into(target, &entry.source_path, source, &mut stdin)?;
            drop(stdin);
            child.wait().map_err(|error| miette!(error))?;
            Ok(())
        }
        PagerResolution::Pager(command) => match spawn_pager(command) {
            Ok(mut child) => {
                let mut stdin = child
                    .stdin
                    .take()
                    .ok_or_else(|| miette!("pager stdin is unavailable"))?;
                run_diff_into(target, &entry.source_path, source, &mut stdin)?;
                drop(stdin);
                child.wait().map_err(|error| miette!(error))?;
                Ok(())
            }
            Err(_) => {
                let mut output = diff_output();
                run_diff_into(target, &entry.source_path, source, &mut output)
            }
        },
        PagerResolution::Stdout => {
            let mut output = diff_output();
            run_diff_into(target, &entry.source_path, source, &mut output)
        }
    }
}

const RENDER_TEMP_DIR: &str = "dotrift-render";

fn render_to_temp(rendered: &[u8]) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(RENDER_TEMP_DIR);
    fs::create_dir_all(&dir)
        .map_err(|error| miette!(error))
        .wrap_err("cannot create diff temp directory")?;
    for _ in 0..3 {
        let name = random_hash()?;
        let path = dir.join(format!("dotrift-{name}.tmp"));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(rendered)
                    .map_err(|error| miette!(error))
                    .wrap_err("cannot write diff temp file")?;
                return Ok(path);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(miette!(error).wrap_err("cannot create diff temp file")),
        }
    }
    Err(miette!("cannot create a unique diff temp file"))
}

fn random_hash() -> Result<String> {
    let mut bytes = [0u8; 16];
    // SAFETY: the buffer is writable and its length matches the request.
    let written = unsafe { libc::getrandom(bytes.as_mut_ptr().cast(), bytes.len(), 0) };
    if written != bytes.len() as isize {
        return Err(miette!("cannot read random bytes for diff temp file"));
    }
    Ok(hash::hash_bytes(&bytes))
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
    Command::new(program)
        .args(parts)
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
    use super::*;
    use tempfile::tempdir;
    use test_case::test_case;

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
