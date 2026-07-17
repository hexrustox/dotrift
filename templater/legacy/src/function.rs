use crate::error::FuncError;
use crate::value::Value;

pub trait FunctionRegistry {
    fn call(&self, name: &str, _args: &[Value]) -> Result<Value, FuncError> {
        Err(FuncError::Undefined(name.to_string()))
    }
}
