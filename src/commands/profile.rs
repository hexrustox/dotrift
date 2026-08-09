use std::io::Write;

use miette::{Result, miette};

use crate::cli::ProfileCommand;
use crate::data::DataFile;
use crate::println_capture;
use crate::state::{StateDatabase, StateLock};

pub fn run(source: Option<&std::path::Path>, command: ProfileCommand) -> Result<()> {
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

fn list(source: &std::path::Path) -> Result<()> {
    let data = DataFile::read(source)?;
    let active = StateDatabase::open_read_only()?
        .map_or_else(|| Ok(Vec::new()), |db| db.active_profiles())?;
    for name in data.profiles.keys() {
        if active.iter().any(|(active_name, _)| active_name == name) {
            println_capture!("{} (active)", name);
        } else {
            println_capture!("{}", name);
        }
    }
    Ok(())
}

fn activate(source: &std::path::Path, name: &str) -> Result<()> {
    let data = DataFile::read(source)?;
    if !data.profiles.contains_key(name) {
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

fn show(source: &std::path::Path) -> Result<()> {
    let data = DataFile::read(source)?;
    let active = StateDatabase::open_read_only()?
        .map_or_else(|| Ok(Vec::new()), |db| db.active_profiles())?;
    let context = data.context(&active);
    for (key, value) in context {
        let mut rendered = Vec::new();
        render_value(&value, &mut rendered)?;
        println_capture!(
            "{key:<20}   {}",
            String::from_utf8(rendered).map_err(|error| miette!(error))?
        );
    }
    Ok(())
}

fn render_value(value: &templater::value::Value, output: &mut Vec<u8>) -> Result<()> {
    match value {
        templater::value::Value::Str(value) => output.write_all(value.as_bytes()),
        templater::value::Value::Int(value) => write!(output, "{value}"),
        templater::value::Value::Bool(value) => write!(output, "{value}"),
        templater::value::Value::List(_) | templater::value::Value::Map(_) => {
            write!(output, "{}", Canonical(value))
        }
    }
    .map_err(|error| miette!(error))
}

struct Canonical<'a>(&'a templater::value::Value);
impl std::fmt::Display for Canonical<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            templater::value::Value::Str(value) => write!(
                formatter,
                "\"{}\"",
                value.replace('\\', "\\\\").replace('"', "\\\"")
            ),
            templater::value::Value::Int(value) => write!(formatter, "{value}"),
            templater::value::Value::Bool(value) => write!(formatter, "{value}"),
            templater::value::Value::List(values) => {
                write!(formatter, "[")?;
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        write!(formatter, ", ")?;
                    }
                    write!(formatter, "{}", Canonical(value))?;
                }
                write!(formatter, "]")
            }
            templater::value::Value::Map(values) => {
                write!(formatter, "{{")?;
                for (index, (key, value)) in values.iter().enumerate() {
                    if index > 0 {
                        write!(formatter, ", ")?;
                    }
                    write!(
                        formatter,
                        "\"{}\": {}",
                        key.replace('\\', "\\\\").replace('"', "\\\""),
                        Canonical(value)
                    )?;
                }
                write!(formatter, "}}")
            }
        }
    }
}
