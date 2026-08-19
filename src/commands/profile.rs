use std::path::Path;

use miette::{Result, miette};

use crate::{
    cli::ProfileCommand,
    data::DataFile,
    println_capture,
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
    let data = DataFile::read(source)?;
    let active = StateDatabase::open_read_only()?
        .map_or_else(|| Ok(Vec::new()), |db| db.active_profiles())?;
    for name in data.profile.keys() {
        if active.iter().any(|(active_name, _)| active_name == name) {
            println_capture!("{} (active)", name);
        } else {
            println_capture!("{}", name);
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
    println_capture!("profile `{name}` activated");
    Ok(())
}

fn deactivate(name: &str) -> Result<()> {
    {
        let _lock = StateLock::acquire()?;
        if !StateDatabase::open()?.deactivate_profile(name)? {
            return Err(miette!("profile `{name}` is not active"));
        }
    }
    println_capture!("profile `{name}` deactivated");
    Ok(())
}

fn show(source: &Path) -> Result<()> {
    let data = DataFile::read(source)?;
    let active = StateDatabase::open_read_only()?
        .map_or_else(|| Ok(Vec::new()), |db| db.active_profiles())?;
    let context = data.context(&active);
    let max = context.keys().map(|s| s.len()).max().unwrap_or(0);
    for (key, value) in context {
        let mut rendered = Vec::new();
        value
            .write_top(&mut rendered)
            .map_err(|error| miette!(error))?;
        println_capture!(
            "{key:<max$}   {}",
            String::from_utf8(rendered).map_err(|error| miette!(error))?,
        );
    }
    Ok(())
}
