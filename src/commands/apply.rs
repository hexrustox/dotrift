use std::{
    collections::HashSet,
    fs,
    hash::Hasher,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use miette::{Result, WrapErr, miette};
use strum::EnumIter;
use tui::prompt::{PromptError, PromptOption};
use twox_hash::XxHash64;

use crate::config::{self, DeployType};
use crate::hash;
use crate::managed;
use crate::state::{Kind, StateDatabase, StateLock, StateRecord};
use crate::template;
use crate::{ExitStatus, println_capture};

/// Reconciles the desired deployment with the target directory.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApplyOptions {
    pub clean_up: bool,
    pub prune_empty_dirs: bool,
    pub dry_run: bool,
    pub quiet: bool,
    pub verbose: bool,
}

pub fn run(source: &Path, target_override: Option<std::path::PathBuf>) -> Result<ExitStatus> {
    run_with_options(source, target_override, ApplyOptions::default())
}

pub fn run_with_options(
    source: &Path,
    target_override: Option<std::path::PathBuf>,
    options: ApplyOptions,
) -> Result<ExitStatus> {
    let _lock = StateLock::acquire()?;
    let deployment = config::read(source, target_override)?;
    let target = &deployment.target_directory;

    if fs::symlink_metadata(target)
        .map(|metadata| !metadata.file_type().is_dir())
        .unwrap_or(false)
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
    context: &std::collections::HashMap<String, templater::value::Value>,
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
    desired: &HashSet<std::path::PathBuf>,
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
        let parent = path
            .parent()
            .ok_or_else(|| miette!("stale target path has no parent"))?;
        if has_symlink_component(target_root, parent)? {
            if !dry_run {
                database.remove(path)?;
            }
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

fn report_dry_run_pruning(
    target_root: &Path,
    removals: &HashSet<std::path::PathBuf>,
) -> Result<()> {
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
            if has_symlink_component(target_root, &directory)? {
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

fn has_symlink_component(target_root: &Path, path: &Path) -> Result<bool> {
    let relative = path
        .strip_prefix(target_root)
        .map_err(|_| miette!("path is outside target directory"))?;
    let mut current = target_root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Ok(true),
            Ok(_) => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    || error.kind() == std::io::ErrorKind::NotADirectory =>
            {
                return Ok(false);
            }
            Err(error) => return Err(miette!(error).wrap_err("cannot inspect target parent")),
        }
    }
    Ok(false)
}

fn prune_parents(target_root: &Path, removed_path: &Path, verbose: bool) -> Result<usize> {
    let mut current = removed_path.parent();
    let mut count = 0;
    while let Some(parent) = current {
        if parent == target_root || !parent.starts_with(target_root) {
            break;
        }
        if has_symlink_component(target_root, parent)? {
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
        println!(
            "Cannot deploy {} {}: {} {} is already present.",
            path_kind(&entry.source_path)?,
            entry.source_path.display(),
            path_kind(obstruction)?,
            obstruction.display()
        );
        let question = "How would you like to proceed?";
        let should_show_diff = fs::metadata(&entry.source_path)
            .is_ok_and(|metadata| metadata.is_file())
            && fs::metadata(obstruction).is_ok_and(|metadata| metadata.is_file());
        tui::prompt::SelectPrompt::new()
            .question(question)
            .filter(move |choice| should_show_diff || *choice != ObstructionChoice::ViewDiff)
            .interact()
    }
}

#[cfg(not(any(test, feature = "testing")))]
fn path_kind(path: &Path) -> std::io::Result<&'static str> {
    Ok(if fs::symlink_metadata(path)?.is_dir() {
        "directory"
    } else {
        "file"
    })
}

fn view_diff(
    entry: &config::DeploymentEntry,
    target: &Path,
    context: &std::collections::HashMap<String, templater::value::Value>,
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
            let mut stdin = child.stdin.take().expect("piped stdin");
            run_diff_into(target, &entry.source_path, source, &mut stdin)?;
            drop(stdin);
            child.wait().map_err(|error| miette!(error))?;
            Ok(())
        }
        PagerResolution::Pager(command) => match spawn_pager(command) {
            Ok(mut child) => {
                let mut stdin = child.stdin.take().expect("piped stdin");
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
    let mut hasher = XxHash64::with_seed(0);
    hasher.write(&bytes);
    Ok(format!("{:016x}", hasher.finish()))
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
    let mut stdout = child.stdout.take().expect("piped stdout");
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
    let program = parts.next().expect("non-empty pager command");
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

fn parent_obstruction(
    target_root: &Path,
    target_path: &Path,
) -> Result<Option<std::path::PathBuf>> {
    let parent = target_path
        .parent()
        .ok_or_else(|| miette!("target path has no parent"))?;
    let relative = parent
        .strip_prefix(target_root)
        .map_err(|_| miette!("target path is outside target directory"))?;
    let mut current = target_root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => return Ok(Some(current)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
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
    use std::collections::{HashMap, HashSet};
    use std::path::{Path, PathBuf};

    use super::*;
    use tempfile::tempdir;
    use test_case::test_case;

    fn test_db() -> (tempfile::TempDir, StateDatabase) {
        let tmp = tempdir().unwrap();
        let database = StateDatabase::open_at(&tmp.path().join("db")).unwrap();
        (tmp, database)
    }

    fn prompt_count() -> usize {
        PROMPT_COUNT.with(|count| *count.borrow())
    }

    #[test_case(|t| t.join("file") => None; "direct_child_has_no_obstruction")]
    #[test_case(|t| {
        fs::create_dir_all(t.join("a/b/c")).unwrap();
        t.join("a/b/c/file")
    } => None; "all_parent_dirs_are_clear")]
    #[test_case(|t| {
        fs::create_dir(t.join("a")).unwrap();
        fs::write(t.join("a/b"), b"").unwrap();
        t.join("a/b/c/file")
    } => Some(PathBuf::from("a/b")); "intermediate_parent_is_a_file")]
    #[test_case(|t| {
        fs::create_dir(t.join("x")).unwrap();
        symlink(t.join("x"), t.join("a")).unwrap();
        t.join("a/b/file")
    } => Some(PathBuf::from("a")); "symlink_to_dir_is_an_obstruction")]
    #[test_case(|t| {
        fs::create_dir(t.join("a")).unwrap();
        t.join("a/b/c/file")
    } => None; "missing_parent_subtree_short_circuits")]
    #[test_case(|t| t.join("a/b/file") => None; "empty_root_target_has_missing_parent")]
    #[test_case(|_| PathBuf::from("/outside/target/file") => panics ""; "target_outside_root")]
    #[test_case(|_| PathBuf::from("/") => panics ""; "target_has_no_parent")]
    fn parent_obstruction_test(setup: impl Fn(&Path) -> PathBuf) -> Option<PathBuf> {
        let tmp = tempdir().unwrap();
        parent_obstruction(tmp.path(), &setup(tmp.path()))
            .unwrap()
            .map(|obstruction| obstruction.strip_prefix(tmp.path()).unwrap().to_path_buf())
    }

    #[test_case(
        |db, t| {
            fs::write(t.join("file"), b"data").unwrap();
            db.put(&crate::record!(f, t.join("file"), hash::hash_bytes(b"data"))).unwrap();
            t.join("file")
        },
        |db, t| {
            assert!(fs::symlink_metadata(t.join("file")).is_err());
            assert_eq!(db.record(&t.join("file")).unwrap(), None);
        };
        "removes_regular_file"
    )]
    #[test_case(
        |db, t| {
            fs::write(t.join("real"), b"data").unwrap();
            symlink(t.join("real"), t.join("link")).unwrap();
            db.put(&crate::record!(s, t.join("link"), t.join("real"))).unwrap();
            t.join("link")
        },
        |db, t| {
            assert!(fs::symlink_metadata(t.join("link")).is_err());
            assert!(fs::symlink_metadata(t.join("real")).is_ok());
            assert_eq!(db.record(&t.join("link")).unwrap(), None);
        };
        "removes_symlink_but_not_target"
    )]
    #[test_case(
        |_db, t| {
            fs::create_dir(t.join("target")).unwrap();
            symlink(t.join("target"), t.join("link")).unwrap();
            t.join("link")
        },
        |_db, t| {
            assert!(fs::symlink_metadata(t.join("link")).is_err());
            assert!(fs::symlink_metadata(t.join("target")).is_ok());
        };
        "removes_symlink_to_dir_without_following"
    )]
    #[test_case(
        |db, t| {
            fs::create_dir_all(t.join("a/b/c")).unwrap();
            fs::write(t.join("a/b/c/deep"), b"").unwrap();
            db.put(&crate::record!(f, t.join("a"), "h1")).unwrap();
            db.put(&crate::record!(f, t.join("a/b/c"), "h2")).unwrap();
            db.put(&crate::record!(f, t.join("a/b/c/deep"), "h3")).unwrap();
            t.join("a")
        },
        |db, t| {
            assert!(fs::symlink_metadata(t.join("a")).is_err());
            for relative in ["a", "a/b/c", "a/b/c/deep"] {
                assert_eq!(db.record(&t.join(relative)).unwrap(), None);
            }
        };
        "removes_nested_directory_tree"
    )]
    #[test_case(
        |db, t| {
            fs::create_dir(t.join("a")).unwrap();
            fs::write(t.join("real"), b"").unwrap();
            symlink(t.join("real"), t.join("a/link")).unwrap();
            db.put(&crate::record!(f, t.join("a"), "h1")).unwrap();
            db.put(&crate::record!(s, t.join("a/link"), t.join("real"))).unwrap();
            t.join("a")
        },
        |db, t| {
            assert!(fs::symlink_metadata(t.join("a")).is_err());
            assert!(fs::symlink_metadata(t.join("real")).is_ok());
            assert_eq!(db.record(&t.join("a")).unwrap(), None);
            assert_eq!(db.record(&t.join("a/link")).unwrap(), None);
        };
        "removes_tree_containing_symlink_without_following"
    )]
    #[test_case(
        |db, t| {
            fs::create_dir_all(t.join("a")).unwrap();
            fs::write(t.join("a/child"), b"").unwrap();
            fs::write(t.join("other"), b"").unwrap();
            db.put(&crate::record!(f, t.join("a"), "h1")).unwrap();
            t.join("a")
        },
        |db, t| {
            assert!(fs::symlink_metadata(t.join("a")).is_err());
            assert!(fs::symlink_metadata(t.join("other")).is_ok());
            assert_eq!(db.record(&t.join("a")).unwrap(), None);
        };
        "removes_tree_but_preserves_sibling"
    )]
    #[test_case(
        |_db, t| {
            fs::create_dir(t.join("empty")).unwrap();
            t.join("empty")
        },
        |_db, t| assert!(fs::symlink_metadata(t.join("empty")).is_err());
        "removes_empty_directory"
    )]
    #[test_case(
        |_db, t| t.join("missing"),
        |_db, _t| {}
        => panics ""
        ; "missing_path_is_an_error"
    )]
    fn remove_path_test(
        setup: impl Fn(&mut StateDatabase, &Path) -> PathBuf,
        assert: impl Fn(&StateDatabase, &Path),
    ) {
        let (_tmp, mut database) = test_db();
        let path = setup(&mut database, _tmp.path());
        remove_path(&database, &path).unwrap();
        assert(&database, _tmp.path())
    }

    macro_rules! entry {
        ($source:expr, $target:expr, $deploy_type:ident) => {
            config::DeploymentEntry {
                source_path: $source,
                target_path: $target,
                deploy_type: DeployType::$deploy_type,
                mode: None,
            }
        };
        ($source:expr, $target:expr, $deploy_type:ident, $mode:expr) => {
            config::DeploymentEntry {
                source_path: $source,
                target_path: $target,
                deploy_type: DeployType::$deploy_type,
                mode: Some(config::DeployMode::try_from($mode).unwrap()),
            }
        };
    }

    #[test_case(
        |_db, t| {
            let source = t.join("src");
            fs::write(&source, b"content").unwrap();
            entry!(source, t.join("target"), Symlink)
        },
        |db, t| {
            let source = t.join("src");
            let metadata = fs::symlink_metadata(t.join("target")).unwrap();
            assert!(metadata.file_type().is_symlink());
            assert_eq!(fs::read_link(t.join("target")).unwrap(), source);
            let record = db.record(&t.join("target")).unwrap().unwrap();
            assert_eq!(record.kind, Kind::Symlink);
            assert_eq!(record.source_path, source);
            assert_eq!(record.content_hash, None);
        }
        => EntryResult::Deployed
        ; "deploys_symlink"
    )]
    #[test_case(
        |_db, t| {
            let source = t.join("src");
            fs::write(&source, b"copy").unwrap();
            entry!(source, t.join("target"), Copy)
        },
        |db, t| {
            assert_eq!(fs::read(t.join("target")).unwrap(), b"copy");
            let metadata = fs::symlink_metadata(t.join("target")).unwrap();
            assert!(metadata.file_type().is_file());
            let record = db.record(&t.join("target")).unwrap().unwrap();
            assert_eq!(record.kind, Kind::File);
            assert_eq!(record.content_hash, Some(hash::hash_bytes(b"copy")));
        }
        => EntryResult::Deployed
        ; "deploys_copy"
    )]
    #[test_case(
        |_db, t| {
            let source = t.join("src");
            fs::write(&source, br#"{{"rendered"}}"#).unwrap();
            entry!(source, t.join("target"), Template)
        },
        |db, t| {
            assert_eq!(fs::read(t.join("target")).unwrap(), b"rendered");
            let metadata = fs::symlink_metadata(t.join("target")).unwrap();
            assert!(metadata.file_type().is_file());
            let record = db.record(&t.join("target")).unwrap().unwrap();
            assert_eq!(record.kind, Kind::File);
            assert_eq!(record.content_hash, Some(hash::hash_bytes(b"rendered")));
        }
        => EntryResult::Deployed
        ; "deploys_template"
    )]
    #[test_case(
        |_db, t| {
            let source = t.join("src");
            fs::write(&source, b"copy").unwrap();
            entry!(source, t.join("target"), Copy, 0o600)
        },
        |_db, t| {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::read(t.join("target")).unwrap(), b"copy");
            let mode = fs::metadata(t.join("target")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        => EntryResult::Deployed
        ; "deploys_copy_with_mode"
    )]
    #[test_case(
        |_db, t| {
            let source = t.join("src");
            fs::write(&source, b"deep").unwrap();
            entry!(source, t.join("a/b/c/file"), Copy)
        },
        |db, t| {
            assert_eq!(fs::read(t.join("a/b/c/file")).unwrap(), b"deep");
            assert!(db.record(&t.join("a/b/c/file")).unwrap().is_some());
            assert_eq!(db.managed_paths().iter().len(), 1);
        }
        => EntryResult::Deployed
        ; "deploys_into_nested_missing_dirs"
    )]
    #[test_case(
        |db, t| {
            let target = t.join("target");
            fs::write(&target, b"old").unwrap();
            db.put(&crate::record!(f, target.clone(), hash::hash_bytes(b"old"))).unwrap();
            let source = t.join("src");
            fs::write(&source, b"new").unwrap();
            entry!(source, target, Copy)
        },
        |db, t| {
            assert_eq!(fs::read(t.join("target")).unwrap(), b"new");
            let record = db.record(&t.join("target")).unwrap().unwrap();
            assert_eq!(record.content_hash, Some(hash::hash_bytes(b"new")));
        }
        => EntryResult::Replaced
        ; "replaces_managed_file_target"
    )]
    #[test_case(
        |db, t| {
            let target = t.join("target");
            let old_source = t.join("old-src");
            fs::write(&old_source, b"old").unwrap();
            symlink(&old_source, &target).unwrap();
            db.put(&crate::record!(s, target.clone(), old_source.clone())).unwrap();
            let source = t.join("src");
            fs::write(&source, b"new").unwrap();
            entry!(source, target, Symlink)
        },
        |db, t| {
            assert_eq!(fs::read_link(t.join("target")).unwrap(), t.join("src"));
            let record = db.record(&t.join("target")).unwrap().unwrap();
            assert_eq!(record.kind, Kind::Symlink);
            assert_eq!(record.source_path, t.join("src"));
        }
        => EntryResult::Replaced
        ; "replaces_managed_symlink_target"
    )]
    #[test_case(
        |_db, t| {
            set_prompt_choice(ObstructionChoice::Replace);
            let target = t.join("target");
            fs::write(&target, b"old").unwrap();
            let source = t.join("src");
            fs::write(&source, b"new").unwrap();
            entry!(source, target, Copy)
        },
        |db, t| {
            assert_eq!(fs::read(t.join("target")).unwrap(), b"new");
            assert!(db.record(&t.join("target")).unwrap().is_some());
        }
        => EntryResult::Replaced
        ; "replaces_unmanaged_target_via_prompt"
    )]
    #[test_case(
        |_db, t| {
            set_prompt_choice(ObstructionChoice::Skip);
            let target = t.join("target");
            fs::write(&target, b"old").unwrap();
            let source = t.join("src");
            fs::write(&source, b"new").unwrap();
            entry!(source, target, Copy)
        },
        |db, t| {
            assert_eq!(fs::read(t.join("target")).unwrap(), b"old");
            assert_eq!(db.record(&t.join("target")).unwrap(), None);
        }
        => EntryResult::Skipped
        ; "skips_unmanaged_target_via_prompt"
    )]
    #[test_case(
        |_db, t| {
            set_prompt_choice(ObstructionChoice::Skip);
            fs::create_dir(t.join("a")).unwrap();
            fs::write(t.join("a/b"), b"").unwrap();
            let source = t.join("src");
            fs::write(&source, b"new").unwrap();
            entry!(source, t.join("a/b/file"), Copy)
        },
        |db, t| {
            assert!(fs::symlink_metadata(t.join("a/b")).unwrap().is_file());
            assert!(fs::symlink_metadata(t.join("a/b/file")).is_err());
            assert_eq!(db.record(&t.join("a/b/file")).unwrap(), None);
        }
        => EntryResult::Skipped
        ; "skips_parent_obstruction_via_prompt"
    )]
    #[test_case(
        |_db, t| {
            set_prompt_choice(ObstructionChoice::Replace);
            fs::create_dir(t.join("a")).unwrap();
            fs::write(t.join("a/b"), b"").unwrap();
            let source = t.join("src");
            fs::write(&source, b"new").unwrap();
            entry!(source, t.join("a/b/file"), Copy)
        },
        |db, t| {
            assert!(fs::metadata(t.join("a/b")).unwrap().is_dir());
            assert_eq!(fs::read(t.join("a/b/file")).unwrap(), b"new");
            assert!(db.record(&t.join("a/b/file")).unwrap().is_some());
        }
        => EntryResult::Replaced
        ; "removes_parent_obstruction_via_prompt"
    )]
    #[test_case(
        |_db, t| {
            set_prompt_choice(ObstructionChoice::Replace);
            fs::create_dir_all(t.join("a")).unwrap();
            fs::create_dir(t.join("real")).unwrap();
            symlink(t.join("real"), t.join("a/b")).unwrap();
            let source = t.join("src");
            fs::write(&source, b"new").unwrap();
            entry!(source, t.join("a/b/file"), Copy)
        },
        |db, t| {
            assert!(fs::metadata(t.join("a/b")).unwrap().is_dir());
            assert_eq!(fs::read(t.join("a/b/file")).unwrap(), b"new");
            assert!(db.record(&t.join("a/b/file")).unwrap().is_some());
        }
        => EntryResult::Replaced
        ; "removes_symlink_parent_obstruction_via_prompt"
    )]
    #[test_case(
        |_db, t| {
            set_prompt_choice(ObstructionChoice::Replace);
            fs::create_dir_all(t.join("a")).unwrap();
            fs::create_dir(t.join("real")).unwrap();
            fs::write(t.join("real/file"), b"old").unwrap();
            symlink(t.join("real"), t.join("a/b")).unwrap();
            let source = t.join("src");
            fs::write(&source, b"new").unwrap();
            entry!(source, t.join("a/b/file"), Copy)
        },
        |db, t| {
            assert!(fs::metadata(t.join("a/b")).unwrap().is_dir());
            assert_eq!(fs::read(t.join("a/b/file")).unwrap(), b"new");
            assert_eq!(fs::read(t.join("real/file")).unwrap(), b"old");
            assert!(db.record(&t.join("a/b/file")).unwrap().is_some());
        }
        => EntryResult::Replaced
        ; "symlink_parent_with_existing_leaf_is_replaced_as_obstruction"
    )]
    #[test_case(
        |_db, t| {
            let source = t.join("src");
            symlink(t.join("missing"), &source).unwrap();
            entry!(source, t.join("target"), Copy)
        },
        |db, t| {
            assert!(fs::symlink_metadata(t.join("target")).is_err());
            assert_eq!(db.record(&t.join("target")).unwrap(), None);
        }
        => panics ""
        ; "source_is_a_dangling_symlink"
    )]
    #[test_case(
        |_db, t| {
            let source = t.join("srcdir");
            fs::create_dir(&source).unwrap();
            entry!(source, t.join("target"), Copy)
        },
        |db, t| {
            assert!(fs::symlink_metadata(t.join("target")).is_err());
            assert_eq!(db.record(&t.join("target")).unwrap(), None);
        }
        => panics ""
        ; "source_is_not_a_regular_file"
    )]
    #[test_case(
        |_db, t| {
            let source = t.join("src");
            fs::write(&source, b"{{ missing }}").unwrap();
            entry!(source, t.join("target"), Template)
        },
        |db, t| {
            assert!(fs::symlink_metadata(t.join("target")).is_err());
            assert_eq!(db.record(&t.join("target")).unwrap(), None);
        }
        => panics ""
        ; "template_with_undefined_variable_errors"
    )]
    fn deploy_entry_test(
        setup: impl Fn(&mut StateDatabase, &Path) -> config::DeploymentEntry,
        assert: impl Fn(&StateDatabase, &Path),
    ) -> EntryResult {
        let (_tmp, mut database) = test_db();
        let entry = setup(&mut database, _tmp.path());
        let mut replace_all = false;
        let result = deploy_entry(
            &database,
            _tmp.path(),
            &entry,
            &HashMap::new(),
            &mut replace_all,
        );
        assert(&database, _tmp.path());
        result.unwrap()
    }

    #[test_case(
        |_db, t| {
            let target_a = t.join("target_a");
            fs::write(&target_a, b"old-a").unwrap();
            let source_a = t.join("src_a");
            fs::write(&source_a, b"new-a").unwrap();
            let target_b = t.join("target_b");
            fs::write(&target_b, b"old-b").unwrap();
            let source_b = t.join("src_b");
            fs::write(&source_b, b"new-b").unwrap();
            vec![
                entry!(source_a, target_a, Copy),
                entry!(source_b, target_b, Copy),
            ]
        },
        |db, t| {
            assert_eq!(fs::read(t.join("target_a")).unwrap(), b"new-a");
            assert_eq!(fs::read(t.join("target_b")).unwrap(), b"new-b");
            assert!(db.record(&t.join("target_a")).unwrap().is_some());
            assert!(db.record(&t.join("target_b")).unwrap().is_some());
        }
        => vec![EntryResult::Replaced, EntryResult::Replaced]
        ; "replace_all_latches_across_entries"
    )]
    #[test_case(
        |_db, t| {
            fs::create_dir(t.join("a")).unwrap();
            fs::write(t.join("a/b"), b"").unwrap();
            let source = t.join("src");
            fs::write(&source, b"new").unwrap();
            let target_b = t.join("target_b");
            fs::write(&target_b, b"old-b").unwrap();
            let source_b = t.join("src_b");
            fs::write(&source_b, b"new-b").unwrap();
            vec![
                entry!(source, t.join("a/b/file"), Copy),
                entry!(source_b, target_b, Copy),
            ]
        },
        |db, t| {
            assert!(fs::metadata(t.join("a/b")).unwrap().is_dir());
            assert_eq!(fs::read(t.join("a/b/file")).unwrap(), b"new");
            assert_eq!(fs::read(t.join("target_b")).unwrap(), b"new-b");
            assert!(db.record(&t.join("a/b/file")).unwrap().is_some());
            assert!(db.record(&t.join("target_b")).unwrap().is_some());
        }
        => vec![EntryResult::Replaced, EntryResult::Replaced]
        ; "replace_all_latches_from_parent_obstruction"
    )]
    #[test_case(
        |_db, t| {
            let target_a = t.join("target_a");
            fs::write(&target_a, b"old-a").unwrap();
            let source_a = t.join("src_a");
            fs::write(&source_a, b"new-a").unwrap();
            fs::create_dir(t.join("a")).unwrap();
            fs::write(t.join("a/b"), b"").unwrap();
            let source_b = t.join("src_b");
            fs::write(&source_b, b"new-b").unwrap();
            vec![
                entry!(source_a, target_a, Copy),
                entry!(source_b, t.join("a/b/file"), Copy),
            ]
        },
        |db, t| {
            assert_eq!(fs::read(t.join("target_a")).unwrap(), b"new-a");
            assert!(fs::metadata(t.join("a/b")).unwrap().is_dir());
            assert_eq!(fs::read(t.join("a/b/file")).unwrap(), b"new-b");
            assert!(db.record(&t.join("target_a")).unwrap().is_some());
            assert!(db.record(&t.join("a/b/file")).unwrap().is_some());
        }
        => vec![EntryResult::Replaced, EntryResult::Replaced]
        ; "replace_all_latches_via_target_prompt_then_removes_parent_obstruction_without_prompt"
    )]
    fn deploy_entry_replace_all_test(
        setup: impl Fn(&mut StateDatabase, &Path) -> Vec<config::DeploymentEntry>,
        assert: impl Fn(&StateDatabase, &Path),
    ) -> Vec<EntryResult> {
        let (_tmp, mut database) = test_db();
        set_prompt_choice(ObstructionChoice::ReplaceAll);
        let entries = setup(&mut database, _tmp.path());
        let mut replace_all = false;
        let mut results = Vec::new();
        for entry in &entries {
            let result = deploy_entry(
                &database,
                _tmp.path(),
                entry,
                &HashMap::new(),
                &mut replace_all,
            );
            results.push(result.unwrap());
        }
        assert(&database, _tmp.path());
        assert_eq!(prompt_count(), 1);
        results
    }

    #[test_case(
        |t| t.join("missing"),
        |_t| HashSet::new() => true;
        "missing_path_is_empty"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a")).unwrap();
            t.join("a")
        },
        |_t| HashSet::new() => true;
        "existing_empty_dir_is_empty"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a")).unwrap();
            fs::write(t.join("a/child"), b"").unwrap();
            t.join("a")
        },
        |_t| HashSet::new() => false;
        "dir_with_unremoved_child_is_not_empty"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a")).unwrap();
            fs::write(t.join("a/child"), b"").unwrap();
            t.join("a")
        },
        |t| HashSet::from([t.join("a/child")]) => true;
        "dir_with_removed_child_is_empty"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a")).unwrap();
            fs::create_dir(t.join("a/sub")).unwrap();
            t.join("a")
        },
        |t| HashSet::from([t.join("a/sub")]) => true;
        "dir_with_removed_subdir_is_empty"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("parent")).unwrap();
            fs::write(t.join("parent/a"), b"").unwrap();
            t.join("parent/a")
        },
        |_t| HashSet::new() => false;
        "path_is_a_regular_file"
    )]
    fn would_be_empty_test(
        setup: impl Fn(&Path) -> PathBuf,
        removals: impl Fn(&Path) -> HashSet<PathBuf>,
    ) -> bool {
        let tmp = tempdir().unwrap();
        would_be_empty(&setup(tmp.path()), &removals(tmp.path())).unwrap()
    }

    #[test_case(
        |t| t.to_path_buf() => false;
        "target_root_itself"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a/b")).unwrap();
            fs::write(t.join("a/b/file"), b"").unwrap();
            t.join("a/b/file")
        } => false;
        "no_symlinks_anywhere"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("real")).unwrap();
            symlink(t.join("real"), t.join("link")).unwrap();
            t.join("link/deep/file")
        } => true;
        "symlink_as_first_component"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a")).unwrap();
            fs::create_dir(t.join("real")).unwrap();
            symlink(t.join("real"), t.join("a/link")).unwrap();
            t.join("a/link/deep/file")
        } => true;
        "symlink_as_deeper_component"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a")).unwrap();
            fs::write(t.join("a/b"), b"").unwrap();
            t.join("a/b/c/file")
        } => false;
        "intermediate_file_is_not_a_symlink"
    )]
    #[test_case(
        |_t| PathBuf::from("/outside/target/file") => panics "";
        "path_outside_target_root"
    )]
    #[test_case(
        |_t| PathBuf::from("/") => panics "";
        "absolute_path_with_no_strip_prefix"
    )]
    fn has_symlink_component_test(setup: impl Fn(&Path) -> PathBuf) -> bool {
        let tmp = tempdir().unwrap();
        has_symlink_component(tmp.path(), &setup(tmp.path())).unwrap()
    }

    #[test_case(
        |t| {
            fs::write(t.join("file"), b"").unwrap();
            t.join("file")
        } => 0;
        "parent_is_target_root"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a/b")).unwrap();
            t.join("a/b/file")
        } => 2;
        "removes_empty_parent_chain"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a/b/c")).unwrap();
            t.join("a/b/c/file")
        } => 3;
        "removes_deep_empty_chain"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a/b")).unwrap();
            fs::write(t.join("a/b/sibling"), b"").unwrap();
            t.join("a/b/file")
        } => 0;
        "parent_with_sibling_is_not_pruned"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a")).unwrap();
            fs::write(t.join("a/b"), b"").unwrap();
            t.join("a/b/file")
        } => 0;
        "parent_is_a_regular_file"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("real")).unwrap();
            fs::create_dir(t.join("a")).unwrap();
            symlink(t.join("real"), t.join("a/link")).unwrap();
            t.join("a/link/deep/file")
        } => 0;
        "symlink_parent_chain_is_not_pruned"
    )]
    #[test_case(
        |t| t.join("a/b/file") => 0;
        "missing_parent_is_not_pruned"
    )]
    fn prune_parents_test(setup: impl Fn(&Path) -> PathBuf) -> usize {
        let tmp = tempdir().unwrap();
        prune_parents(tmp.path(), &setup(tmp.path()), false).unwrap()
    }

    #[test_case(
        |db, t| {
            fs::write(t.join("stale"), b"data").unwrap();
            db.put(&crate::record!(f, t.join("stale"), hash::hash_bytes(b"data")))
                .unwrap();
        },
        |_t| HashSet::new(),
        ApplyOptions::default(),
        |db, t, removed, pruned| {
            assert_eq!((removed, pruned), (1, 0));
            assert!(fs::symlink_metadata(t.join("stale")).is_err());
            assert_eq!(db.record(&t.join("stale")).unwrap(), None);
        };
        "removes_stale_managed_file"
    )]
    #[test_case(
        |db, t| {
            fs::write(t.join("stale"), b"other").unwrap();
            db.put(&crate::record!(f, t.join("stale"), hash::hash_bytes(b"data")))
                .unwrap();
        },
        |_t| HashSet::new(),
        ApplyOptions::default(),
        |db, t, removed, pruned| {
            assert_eq!((removed, pruned), (0, 0));
            assert_eq!(fs::read(t.join("stale")).unwrap(), b"other");
            assert_eq!(db.record(&t.join("stale")).unwrap(), None);
        };
        "relinquishes_unmanaged_stale_file"
    )]
    #[test_case(
        |db, t| {
            db.put(&crate::record!(f, t.join("missing"), hash::hash_bytes(b"data")))
                .unwrap();
        },
        |_t| HashSet::new(),
        ApplyOptions::default(),
        |db, t, removed, pruned| {
            assert_eq!((removed, pruned), (0, 0));
            assert_eq!(db.record(&t.join("missing")).unwrap(), None);
        };
        "relinquishes_missing_stale_path"
    )]
    #[test_case(
        |db, t| {
            fs::create_dir(t.join("real")).unwrap();
            fs::write(t.join("real/child"), b"x").unwrap();
            symlink(t.join("real"), t.join("link")).unwrap();
            db.put(&crate::record!(f, t.join("link/child"), hash::hash_bytes(b"x")))
                .unwrap();
        },
        |_t| HashSet::new(),
        ApplyOptions::default(),
        |db, t, removed, pruned| {
            assert_eq!((removed, pruned), (0, 0));
            assert_eq!(fs::read(t.join("real/child")).unwrap(), b"x");
            assert_eq!(db.record(&t.join("link/child")).unwrap(), None);
        };
        "relinquishes_path_under_symlink_parent"
    )]
    #[test_case(
        |db, _t| {
            db.put(&crate::record!(f, "/outside/file", "h")).unwrap();
        },
        |_t| HashSet::new(),
        ApplyOptions::default(),
        |db, _t, removed, pruned| {
            assert_eq!((removed, pruned), (0, 0));
            assert!(db.record(Path::new("/outside/file")).unwrap().is_some());
        };
        "skips_path_outside_target_root"
    )]
    #[test_case(
        |db, t| {
            db.put(&crate::record!(f, t, "h")).unwrap();
        },
        |_t| HashSet::new(),
        ApplyOptions::default(),
        |db, t, removed, pruned| {
            assert_eq!((removed, pruned), (0, 0));
            assert!(db.record(t).unwrap().is_some());
        };
        "skips_path_that_is_target_root"
    )]
    #[test_case(
        |db, t| {
            fs::write(t.join("stale"), b"data").unwrap();
            db.put(&crate::record!(f, t.join("stale"), hash::hash_bytes(b"data")))
                .unwrap();
        },
        |t| HashSet::from([t.join("stale")]),
        ApplyOptions::default(),
        |db, t, removed, pruned| {
            assert_eq!((removed, pruned), (0, 0));
            assert!(fs::symlink_metadata(t.join("stale")).is_ok());
            assert!(db.record(&t.join("stale")).unwrap().is_some());
        };
        "skips_path_in_desired"
    )]
    #[test_case(
        |db, t| {
            fs::write(t.join("stale"), b"data").unwrap();
            db.put(&crate::record!(f, t.join("stale"), hash::hash_bytes(b"data")))
                .unwrap();
        },
        |_t| HashSet::new(),
        ApplyOptions {
            dry_run: true,
            ..ApplyOptions::default()
        },
        |db, t, removed, pruned| {
            assert_eq!((removed, pruned), (0, 0));
            assert!(fs::symlink_metadata(t.join("stale")).is_ok());
            assert!(db.record(&t.join("stale")).unwrap().is_some());
        };
        "dry_run_removes_nothing"
    )]
    #[test_case(
        |db, t| {
            fs::create_dir_all(t.join("a/b")).unwrap();
            fs::write(t.join("a/b/file"), b"data").unwrap();
            db.put(&crate::record!(f, t.join("a/b/file"), hash::hash_bytes(b"data")))
                .unwrap();
        },
        |_t| HashSet::new(),
        ApplyOptions {
            prune_empty_dirs: true,
            ..ApplyOptions::default()
        },
        |db, t, removed, pruned| {
            assert_eq!(removed, 1);
            assert_eq!(pruned, 2);
            assert!(fs::symlink_metadata(t.join("a")).is_err());
            assert!(fs::symlink_metadata(t.join("a/b")).is_err());
            assert_eq!(db.record(&t.join("a/b/file")).unwrap(), None);
        };
        "prunes_empty_parent_dirs"
    )]
    fn cleanup_test(
        setup: impl Fn(&mut StateDatabase, &Path),
        desired: impl Fn(&Path) -> HashSet<PathBuf>,
        options: ApplyOptions,
        assert: impl Fn(&StateDatabase, &Path, usize, usize),
    ) {
        let (_tmp, mut database) = test_db();
        setup(&mut database, _tmp.path());
        let (removed, pruned) =
            cleanup(&database, _tmp.path(), &desired(_tmp.path()), options).unwrap();
        assert(&database, _tmp.path(), removed, pruned);
    }
}
