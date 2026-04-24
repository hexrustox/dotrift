// TODO print more
use std::path::Path;

use crate::{
    cli::{GlobalFlags, UnapplyFlags},
    command::util::{clean_up, resolve_target},
    config::Config,
    db::Db,
};

pub fn run(
    global_flags: GlobalFlags,
    db_path: &Path,
    flags: UnapplyFlags,
) -> color_eyre::Result<()> {
    let source_dir = global_flags.source()?;
    let target_override = global_flags.target()?;

    let config = Config::read(&source_dir)?;

    let _ = resolve_target(&source_dir, target_override, &config)?;

    let db = Db::init(db_path)?;
    clean_up(None, &db, flags.dry_run, flags.prune_empty_dirs)?;
    Ok(())
}
