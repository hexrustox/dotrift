use std::path::PathBuf;

use normalize_path::NormalizePath;

use crate::{
    cli::UnapplyFlags,
    command::util::{clean_up, resolve_target, validate_paths},
    config::Config,
    db::Db,
};

pub fn run(
    source_dir: PathBuf,
    target_override: Option<PathBuf>,
    db_path: &PathBuf,
    flags: UnapplyFlags,
) -> color_eyre::Result<()> {
    let config = Config::read(source_dir.clone())?;

    let target_dir = resolve_target(target_override, &config)?.normalize();

    validate_paths(&source_dir, &target_dir)?;

    let db = Db::init(db_path)?;
    clean_up(None, &db, flags.dry_run, flags.prune_empty_dirs)?;
    Ok(())
}
