use std::{collections::HashMap, path::Path};

use miette::Context;
use templater::Value;

use crate::{
    cli::{GlobalFlags, UnapplyFlags},
    command::apply::{build_ignore, resolve_portals},
    command::util::{clean_up, resolve_target},
    config::Config,
    db::Db,
    output,
    templater::{data::TemplateData, function::BuiltinFunctions},
};

pub fn run(global_flags: GlobalFlags, db_path: &Path, flags: UnapplyFlags) -> miette::Result<()> {
    let source_dir = global_flags.source()?;
    let target_override = global_flags.target()?;

    let db = Db::init(db_path)?;
    let mut data = TemplateData::read(&source_dir)?;
    let active_profiles = db.get_active_profiles()?;
    let mut variables: HashMap<String, Value> = data.variable;
    for profile in active_profiles {
        if let Some(vars) = data.profile.remove(&profile.name) {
            variables.extend(vars);
        }
    }
    let functions = BuiltinFunctions::new();
    let (config, _) =
        Config::read_templated(&source_dir, variables, &functions).wrap_err("failed to read config")?;
    let target_dir = resolve_target(&source_dir, target_override, &config)?;

    let ignore_matcher = build_ignore(&config.ignore, &target_dir)?;
    let portal_entries =
        resolve_portals(&source_dir, &target_dir, &config.portal, &ignore_matcher)?;

    let n = clean_up(
        &portal_entries,
        &db,
        flags.dry_run,
        flags.prune_empty_dirs,
        global_flags.verbose,
        true,
    )?;
    if flags.dry_run {
        let label = if n == 1 { "removal" } else { "removals" };
        output::print_summary(format_args!("{} {}", n, label));
    }
    Ok(())
}
