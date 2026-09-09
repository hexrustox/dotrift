//! Rendering templates: the thin dotrift-side wrapper around the templater
//! crate and the per-run render registry.

mod registry;
mod template;

pub(crate) use registry::RenderRegistry;
pub(crate) use template::{render_template, render_template_to};
