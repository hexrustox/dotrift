use std::path::Path;

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use miette::{Result, miette};

pub(super) fn read_ignore(source: &Path) -> Result<Gitignore> {
    let mut builder = GitignoreBuilder::new(source);
    builder
        .add_line(None, "/dotrift.toml")
        .map_err(|error| miette!(error))?;
    builder
        .add_line(None, "/dotrift_data.toml")
        .map_err(|error| miette!(error))?;
    builder
        .add_line(None, "/.dotriftignore")
        .map_err(|error| miette!(error))?;
    let path = source.join(".dotriftignore");
    if path.exists() {
        match builder.add(path) {
            None => {}
            Some(error) => return Err(miette!(error)),
        }
    }
    builder.build().map_err(|error| miette!(error))
}
