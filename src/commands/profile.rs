use std::path::Path;

use miette::{Result, WrapErr, miette};

use super::require_source;
use crate::{
    cli::ProfileCommand,
    config::DataFile,
    platform::Environment,
    report::{Outcome, Reporter},
    state::{StateDatabase, StateLock},
};

pub fn run(
    source: Option<&Path>,
    command: ProfileCommand,
    env: &Environment,
    color: bool,
) -> Result<()> {
    match command {
        ProfileCommand::List => list(require_source("profile", source)?, env, color),
        ProfileCommand::Activate { name } => {
            activate(require_source("profile", source)?, &name, env, color)
        }
        ProfileCommand::Deactivate { name } => deactivate(&name, env, color),
        ProfileCommand::Show => show(require_source("profile", source)?, env, color),
    }
}

fn list(source: &Path, env: &Environment, color: bool) -> Result<()> {
    let report = Reporter::always(color);
    let data = DataFile::read(source)?;
    let active = crate::state::load_active_profiles(env)?;
    for name in data.profile.keys() {
        if active.iter().any(|(active_name, _)| active_name == name) {
            report.line(format_args!(
                "{} {}",
                name,
                report.paint(Outcome::Active, "(active)")
            ));
        } else {
            report.line(format_args!("{name}"));
        }
    }
    Ok(())
}

fn activate(source: &Path, name: &str, env: &Environment, color: bool) -> Result<()> {
    let data = DataFile::read(source)?;
    if !data.profile.contains_key(name) {
        return Err(miette!(
            help = "profiles are defined in `[profile.<name>]` tables in `dotrift_data.toml`",
            "profile `{name}` is not defined",
        ));
    }
    {
        let _lock = StateLock::acquire(env)?;
        StateDatabase::open(env)?.activate_profile(name)?;
    }
    Reporter::always(color).line(format_args!("profile `{name}` activated"));
    Ok(())
}

fn deactivate(name: &str, env: &Environment, color: bool) -> Result<()> {
    {
        let _lock = StateLock::acquire(env)?;
        if !StateDatabase::open(env)?.deactivate_profile(name)? {
            return Err(miette!(
                help = "`dotrift profile list` shows the active profiles",
                "profile `{name}` is not active",
            ));
        }
    }
    Reporter::always(color).line(format_args!("profile `{name}` deactivated"));
    Ok(())
}

fn show(source: &Path, env: &Environment, color: bool) -> Result<()> {
    let report = Reporter::always(color);
    let data = DataFile::read(source)?;
    let active = crate::state::load_active_profiles(env)?;
    let context = data.context(&active);
    let max = context.keys().map(|s| s.len()).max().unwrap_or(0);
    for (key, value) in context {
        let mut rendered = Vec::new();
        value
            .write_top(&mut rendered)
            .map_err(|error| miette!(error))
            .wrap_err_with(|| format!("cannot render value of `{key}`"))?;
        report.line(format_args!(
            "{key:<max$}   {}",
            String::from_utf8(rendered)
                .map_err(|error| miette!(error))
                .wrap_err_with(|| format!("rendered value of `{key}` is not `UTF-8`"))?,
        ));
    }
    Ok(())
}
