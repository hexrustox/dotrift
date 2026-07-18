use std::collections::HashMap;

use templater::{FuncError, FunctionRegistry, Template, Value};

/// Registry where every function is undefined; covers templates that never
/// call functions.
pub struct MockRegistry;

impl FunctionRegistry for MockRegistry {
    fn call(&self, name: &str, _args: &[Value]) -> Result<Value, FuncError> {
        Err(FuncError::Undefined {
            name: name.to_owned(),
        })
    }
}

/// Renders `source` against `variables` and `functions`, panicking on any
/// error. Happy-path wrapper; error tests drive `Template` directly.
pub fn render(
    source: &[u8],
    variables: &HashMap<String, Value>,
    functions: &dyn FunctionRegistry,
) -> Vec<u8> {
    let template = Template::from_bytes(source.to_vec()).expect("parse failed");
    let mut out = Vec::new();
    template
        .render(&mut out, variables, functions)
        .expect("render failed");
    out
}
