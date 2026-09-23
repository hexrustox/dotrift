//! Thin dotrift-side wrapper around the templater crate plus the per-run render registry.

mod builtins;
mod registry;
mod template;

use builtins::Builtins;

pub(crate) use registry::RenderRegistry;
pub(crate) use template::{render_template, render_template_to};
