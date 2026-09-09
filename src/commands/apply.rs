use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use miette::{Result, miette};
use templater::value::Value;

use crate::{
    ExitStatus,
    config::{self, DeployType},
    deployer::{DeployOutcome, Deployer, ReplaceLatch, cleanup, describe_decision},
    environment::Environment,
    global_config::GlobalConfig,
    obstruction_interaction::Interaction,
    prettify_path,
    reconcile::decide,
    render_registry::RenderRegistry,
    report::{Outcome, Reporter},
    state::{StateDatabase, StateLock},
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

pub fn run(
    source: &Path,
    target_override: Option<PathBuf>,
    env: &Environment,
    color: bool,
) -> Result<ExitStatus> {
    run_with_options(source, target_override, ApplyOptions::default(), env, color)
}

pub fn run_with_options(
    source: &Path,
    target_override: Option<PathBuf>,
    options: ApplyOptions,
    env: &Environment,
    color: bool,
) -> Result<ExitStatus> {
    let _lock = StateLock::acquire(env)?;
    let global_config = GlobalConfig::load(env)?;
    let mut registry = RenderRegistry::acquire(env, options.dry_run);
    let deployment = config::read(source, target_override, env, color)?;
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

    let database = StateDatabase::open(env)?;
    let mut entries = deployment.entries.clone();
    entries.sort_by(|left, right| left.target_path.cmp(&right.target_path));
    let report = Reporter::new(color, options.verbose || options.dry_run, options.quiet);

    if options.dry_run {
        for entry in &entries {
            report_dry_run_entry(
                &database,
                target,
                entry,
                &deployment.variable_context,
                &mut registry,
                &global_config,
                &report,
            )?;
        }
        if options.clean_up {
            let desired = deployment
                .entries
                .iter()
                .map(|entry| entry.target_path.clone())
                .collect();
            let _ = cleanup(&database, target, &desired, options, &report)?;
        }
        return Ok(ExitStatus::Success);
    }

    let interaction = Interaction::new(&global_config);
    let mut latch = ReplaceLatch::default();
    let mut deployer = Deployer::new(
        &database,
        target,
        &mut registry,
        &global_config,
        &interaction,
        &mut latch,
    );
    let mut skipped = 0;
    let mut deployed = 0;
    let mut replaced = 0;
    for entry in &entries {
        match deployer.deploy_one(entry, &deployment.variable_context)? {
            DeployOutcome::Deployed => {
                deployed += 1;
                report.outcome_line(format_args!(
                    "{} {}",
                    report.paint(Outcome::Deployed, "deployed"),
                    prettify_path(&entry.target_path).display()
                ));
            }
            DeployOutcome::Replaced => {
                replaced += 1;
                report.outcome_line(format_args!(
                    "{} {}",
                    report.paint(Outcome::Replaced, "replaced"),
                    prettify_path(&entry.target_path).display()
                ));
            }
            DeployOutcome::Skipped => {
                skipped += 1;
                report.outcome_line(format_args!(
                    "{} {}",
                    report.paint(Outcome::Skipped, "skipped"),
                    prettify_path(&entry.target_path).display()
                ));
            }
            DeployOutcome::Cancelled => return Ok(ExitStatus::Cancelled),
        }
    }
    let mut removed = 0;
    let mut pruned = 0;
    if options.clean_up && skipped == 0 {
        let desired = deployment
            .entries
            .iter()
            .map(|entry| entry.target_path.clone())
            .collect();
        (removed, pruned) = cleanup(&database, target, &desired, options, &report)?;
    }
    if options.clean_up {
        report.summary(format_args!(
            "deployed {deployed}, replaced {replaced}, skipped {skipped}, removed {removed}, pruned {pruned}"
        ));
    } else {
        report.summary(format_args!(
            "deployed {deployed}, replaced {replaced}, skipped {skipped}"
        ));
    }
    if skipped > 0 {
        return Ok(ExitStatus::Skipped);
    }
    Ok(ExitStatus::Success)
}

fn report_dry_run_entry(
    database: &StateDatabase,
    target_root: &Path,
    entry: &config::DeploymentEntry,
    context: &HashMap<String, Value>,
    registry: &mut RenderRegistry,
    global_config: &GlobalConfig,
    report: &Reporter,
) -> Result<()> {
    // Preview-only: the latch never engages in dry-run (ADR-0014).
    let decision = decide(
        database,
        target_root,
        entry,
        context,
        registry,
        false,
        global_config.replace_identical(),
    )?;
    let (outcome, word) = describe_decision(&decision);
    let suffix = format_deploy_suffix(entry);
    report.outcome_line(format_args!(
        "{} {} {suffix}",
        report.paint(outcome, word),
        prettify_path(&entry.target_path).display()
    ));
    Ok(())
}

fn format_deploy_suffix(entry: &config::DeploymentEntry) -> String {
    let deploy_type = match entry.deploy_type {
        DeployType::Symlink => "symlink",
        DeployType::Copy => "copy",
        DeployType::Template => "template",
    };
    match entry.mode {
        Some(mode) => format!("[{deploy_type} {:03o}]", u32::from(mode)),
        None => format!("[{deploy_type}]"),
    }
}
