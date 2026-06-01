use std::path::Path;

use crate::{
    cli::{GlobalFlags, UnapplyFlags},
    command::util::{clean_up, resolve_target},
    config::Config,
    db::Db,
    output,
};

pub fn run(global_flags: GlobalFlags, db_path: &Path, flags: UnapplyFlags) -> miette::Result<()> {
    let source_dir = global_flags.source()?;
    let target_override = global_flags.target()?;

    let config = Config::read(&source_dir)?;

    resolve_target(&source_dir, target_override, &config)?;

    let db = Db::init(db_path)?;
    let n = clean_up(
        None,
        &db,
        flags.dry_run,
        flags.prune_empty_dirs,
        global_flags.verbose,
    )?;
    if flags.dry_run {
        let label = if n == 1 { "removal" } else { "removals" };
        output::print_summary(format_args!("{} {}", n, label));
    }
    Ok(())
}
