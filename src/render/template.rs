use std::{
    collections::HashMap,
    io::{self, Write},
    path::Path,
};

use miette::{Report, Result, WrapErr, miette};
use templater::{Template, function::FunctionRegistry, value::Value};

// TODO impl functions
struct NoFunctions;
impl FunctionRegistry for NoFunctions {
    fn call(
        &self,
        name: &str,
        _: &[Value],
    ) -> std::result::Result<Value, templater::error::RegistryError> {
        Err(templater::error::RegistryError::Undefined { name: name.into() })
    }
}

/// Why rendering a template stopped: the template itself, or the sink it was
/// rendered into.
pub(crate) enum RenderFailure {
    /// Reading the source, parsing, or rendering failed; the run must report
    /// this. Carries the final diagnostic.
    Template(Report),
    /// Writing the rendered bytes failed; infrastructure, not the template.
    Sink(io::Error),
}

/// Renders the template at `path` into `writer`, reporting whether the
/// template or the sink failed.
pub(crate) fn render_template_into(
    path: &Path,
    context: &HashMap<String, Value>,
    writer: impl Write,
) -> std::result::Result<(), RenderFailure> {
    let template = Template::from_file(path)
        .map_err(|error| miette!(error))
        .wrap_err_with(|| format!("cannot read `{}`", path.display()))
        .map_err(RenderFailure::Template)?;
    let result = template.render(writer, context, &NoFunctions);
    match result {
        Ok(()) => Ok(()),
        Err(templater::error::Error::Io(error)) => Err(RenderFailure::Sink(error)),
        Err(error) => template
            .report(Err(error))
            .map_err(|error| miette!(error))
            .wrap_err_with(|| format!("cannot render `{}`", path.display()))
            .map_err(RenderFailure::Template),
    }
}

/// Renders the template at `path` into `writer`, treating a sink failure as
/// fatal like any other rendering failure.
pub(crate) fn render_template_to(
    path: &Path,
    context: &HashMap<String, Value>,
    writer: impl Write,
) -> Result<()> {
    match render_template_into(path, context, writer) {
        Ok(()) => Ok(()),
        Err(RenderFailure::Template(report)) => Err(report),
        Err(RenderFailure::Sink(error)) => {
            Err(miette!(error).wrap_err(format!("cannot render `{}`", path.display())))
        }
    }
}

pub(crate) fn render_template(path: &Path, context: &HashMap<String, Value>) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    render_template_to(path, context, &mut output)?;
    Ok(output)
}
