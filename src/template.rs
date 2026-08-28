use std::{collections::HashMap, io::Write, path::Path};

use miette::{Result, WrapErr, miette};
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

pub(crate) fn render_template_to(
    path: &Path,
    context: &HashMap<String, Value>,
    writer: impl Write,
) -> Result<()> {
    let template = Template::from_file(path)
        .map_err(|error| miette!(error))
        .wrap_err_with(|| format!("cannot read `{}`", path.display()))?;
    let result = template.render(writer, context, &NoFunctions);
    template
        .report(result)
        .map_err(|error| miette!(error))
        .wrap_err_with(|| format!("cannot render `{}`", path.display()))
}

pub(crate) fn render_template(path: &Path, context: &HashMap<String, Value>) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    render_template_to(path, context, &mut output)?;
    Ok(output)
}
