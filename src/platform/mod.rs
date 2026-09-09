//! Process-level seams: where dotrift's own files live (resolved from the
//! XDG base directories) and filesystem path helpers.

pub mod environment;
mod paths;

pub use environment::Environment;
pub(crate) use paths::{ensure_absolute, ensure_source_dir, prettify_path};
