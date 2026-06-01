use std::{collections::BTreeMap, path::Path};

use miette::{Result, bail};
use templater::value::Value;

use crate::{cli::GlobalFlags, db::Db, path::data_path, templater::data::TemplateData};

pub fn list(global: &GlobalFlags, db_path: &Path) -> Result<()> {
    let source_dir = global.source()?;
    let data = TemplateData::read(&source_dir)?;

    if data.profile.is_empty() {
        bail!(
            "no profiles defined in `{}`",
            data_path(&source_dir).display()
        );
    }

    let db = Db::init(db_path)?;
    let active_profiles: Vec<String> = db
        .get_active_profiles()?
        .into_iter()
        .map(|p| p.name)
        .collect();

    for profile in active_profiles {
        if data.profile.contains_key(&profile) {
            println!("{profile} (active)");
        } else {
            println!("{profile}");
        }
    }

    Ok(())
}

pub fn activate(global: &GlobalFlags, db_path: &Path, name: &str) -> Result<()> {
    let source_dir = global.source()?;
    let data = TemplateData::read(&source_dir)?;

    if !data.profile.contains_key(name) {
        bail!(
            "profile `{name}` is not defined in `{}`",
            data_path(&source_dir).display()
        );
    }

    let db = Db::init(db_path)?;
    db.activate_profile(name)?;
    eprintln!("profile `{name}` activated");
    Ok(())
}

pub fn deactivate(db_path: &Path, name: &str) -> Result<()> {
    let db = Db::init(db_path)?;
    db.deactivate_profile(name)?;
    eprintln!("profile `{name}` deactivated");
    Ok(())
}

pub fn show(global: &GlobalFlags, db_path: &Path) -> Result<()> {
    let source_dir = global.source()?;
    let mut data = TemplateData::read(&source_dir)?;
    let db = Db::init(db_path)?;
    let active_profiles = db.get_active_profiles()?;

    let mut ctx: BTreeMap<String, Value> = BTreeMap::new();

    for (k, v) in data.variable {
        ctx.insert(k, v);
    }

    for profile in active_profiles {
        if let Some(vars) = data.profile.remove(&profile.name) {
            for (k, v) in vars {
                ctx.insert(k, v);
            }
        }
    }

    if ctx.is_empty() {
        return Ok(());
    }

    let max_key_len = ctx.keys().map(|k| k.len()).max().unwrap_or(0);
    for (key, val) in &ctx {
        let mut buf = Vec::new();
        val.write_to(&mut buf).unwrap();
        println!(
            "{key:<width$}  {}",
            String::from_utf8_lossy(&buf),
            width = max_key_len
        );
    }

    Ok(())
}
