use std::path::Path;

use miette::Result;

use crate::internal_error;

pub mod apply;
pub mod init;
pub mod profile;
pub mod status;

pub fn require_source<'a>(command: &'a str, source: Option<&'a Path>) -> Result<&'a Path> {
    source.ok_or_else(|| internal_error(format!("missing source directory for `{command}`")))
}
