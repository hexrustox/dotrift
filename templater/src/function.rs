use crate::error::FuncError;
use crate::value::Value;

pub trait FunctionRegistry {
    fn call(&self, name: &str, args: &[Value]) -> Result<Value, FuncError>;
}
