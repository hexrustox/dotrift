use std::path::Path;

use miette::{Result, miette};

use crate::{
    cli::ProfileCommand,
    data::DataFile,
    report::{Outcome, Reporter},
    state::{StateDatabase, StateLock},
};

pub fn run(source: Option<&Path>, command: ProfileCommand) -> Result<()> {
    match command {
        ProfileCommand::List => {
            list(source.ok_or_else(|| miette!("source directory is required"))?)
        }
        ProfileCommand::Activate { name } => activate(
            source.ok_or_else(|| miette!("source directory is required"))?,
            &name,
        ),
        ProfileCommand::Deactivate { name } => deactivate(&name),
        ProfileCommand::Show => {
            show(source.ok_or_else(|| miette!("source directory is required"))?)
        }
    }
}

fn list(source: &Path) -> Result<()> {
    let report = Reporter::always();
    let data = DataFile::read(source)?;
    let active = crate::state::load_active_profiles()?;
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

fn activate(source: &Path, name: &str) -> Result<()> {
    let data = DataFile::read(source)?;
    if !data.profile.contains_key(name) {
        return Err(miette!("profile `{name}` is not defined"));
    }
    {
        let _lock = StateLock::acquire()?;
        StateDatabase::open()?.activate_profile(name)?;
    }
    Reporter::always().line(format_args!("profile `{name}` activated"));
    Ok(())
}

fn deactivate(name: &str) -> Result<()> {
    {
        let _lock = StateLock::acquire()?;
        if !StateDatabase::open()?.deactivate_profile(name)? {
            return Err(miette!("profile `{name}` is not active"));
        }
    }
    Reporter::always().line(format_args!("profile `{name}` deactivated"));
    Ok(())
}

fn show(source: &Path) -> Result<()> {
    let report = Reporter::always();
    let data = DataFile::read(source)?;
    let active = crate::state::load_active_profiles()?;
    let context = data.context(&active);
    let max = context.keys().map(|s| s.len()).max().unwrap_or(0);
    for (key, value) in context {
        let mut rendered = Vec::new();
        value
            .write_top(&mut rendered)
            .map_err(|error| miette!(error))?;
        report.line(format_args!(
            "{key:<max$}   {}",
            String::from_utf8(rendered).map_err(|error| miette!(error))?,
        ));
    }
    Ok(())
}
