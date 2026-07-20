use std::collections::HashMap;

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

/// A variable scope shared by several test cases.
#[allow(dead_code)]
pub fn var_scope() -> HashMap<String, Value> {
    HashMap::from([
        ("name".to_string(), Value::Str("world".to_string())),
        ("count".to_string(), Value::Int(42)),
        ("neg".to_string(), Value::Int(-5)),
        ("flag".to_string(), Value::Bool(true)),
        ("off".to_string(), Value::Bool(false)),
    ])
}
