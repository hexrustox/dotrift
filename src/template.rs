use std::{collections::HashMap, path::Path};

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

pub fn render_template(path: &Path, context: &HashMap<String, Value>) -> Result<Vec<u8>> {
    let template = Template::from_file(path)
        .map_err(|error| miette!(error))
        .wrap_err_with(|| format!("cannot read `{}`", path.display()))?;
    let mut output = Vec::new();
    let result = template.render(&mut output, context, &NoFunctions);
    template
        .report(result)
        .map_err(|error| miette!(error))
        .wrap_err_with(|| format!("cannot render `{}`", path.display()))?;
    Ok(output)
}
