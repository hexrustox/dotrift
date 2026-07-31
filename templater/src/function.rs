use crate::{RegistryError, Value};

/// The host-provided table of named functions, resolved at render time.
pub trait FunctionRegistry {
    /// Invokes the function named `name` with the evaluated `args`.
    ///
    /// Returns the function's result on success, or a [`RegistryError`] if the
    /// function is undefined, received the wrong number of arguments, or
    /// received an argument of an incompatible type.
    fn call(&self, name: &str, args: &[Value]) -> std::result::Result<Value, RegistryError>;
}
