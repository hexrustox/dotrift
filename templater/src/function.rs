use crate::{FuncError, Value};

/// The host-provided table of named functions, resolved at render time.
pub trait FunctionRegistry {
    fn call(&self, name: &str, args: &[Value]) -> std::result::Result<Value, FuncError>;
}
