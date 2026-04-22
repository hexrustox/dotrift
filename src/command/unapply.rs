use std::path::{Path, PathBuf};

use crate::{
    cli::UnapplyFlags,
    command::util::{clean_up, resolve_target},
    config::Config,
    db::Db,
};

pub fn run(
    source_dir: PathBuf,
    target_override: Option<PathBuf>,
    db_path: &Path,
    flags: UnapplyFlags,
) -> color_eyre::Result<()> {
    let config = Config::read(&source_dir)?;

    let _ = resolve_target(&source_dir, target_override, &config)?;

    let db = Db::init(db_path)?;
    clean_up(None, &db, flags.dry_run, flags.prune_empty_dirs)?;
    Ok(())
}
