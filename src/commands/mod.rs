pub mod apply;
pub mod init;
pub mod profile;
pub mod status;

use std::path::Path;

pub fn require_source<'a>(command: &'a str, source: Option<&'a Path>) -> miette::Result<&'a Path> {
    source.ok_or_else(|| crate::internal_error(format!("missing source directory for `{command}`")))
}
