use std::path::{Path, PathBuf};

use miette::{Result, miette};
use tui::pager::{self, PagerArgs};

use crate::{
    command::util::{PathLiteral, to_absolute_path},
    db::Db,
};

pub fn run(path: PathBuf, db_path: &Path) -> Result<()> {
    let path = to_absolute_path(&path)?;

    let db = Db::init(db_path)?;

    let entry = db
        .get_entry(&path)?
        .ok_or_else(|| miette!("`{}` is not managed", path.display()))?;

    if !entry.source_path.path_exists() {
        return Err(miette!(
            "source file `{}` not found",
            entry.source_path.display()
        ));
    }

    if !path.path_exists() {
        return Err(miette!("target file `{}` not found", path.display()));
    }

    pager::run(PagerArgs::Diff {
        source: &entry.source_path,
        target: &path,
    })
    .map_err(|e| miette!(e))?;

    Ok(())
}
