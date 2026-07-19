use templater::{FuncError, FunctionRegistry, Value};

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
