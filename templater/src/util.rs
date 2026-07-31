use std::ops::Range;

use miette::SourceSpan;

/// Byte classification shared by the scanner and parser.
///
/// Inner-edge ASCII whitespace inside a `{{ ... }}` body: space, tab, and
/// `\n` only. `\r` is intentionally *not* classified as whitespace — per
/// spec, only `\n` is a line terminator; `\r` is ordinary text and must
/// survive trimming so it is not silently lost in CRLF sources.
pub(crate) fn is_whitespace(b: u8) -> bool {
    b == b' ' || b == b'\t' || b == b'\n'
}

/// Builds a miette `SourceSpan` from a byte range in source coordinates.
pub(crate) fn source_span(range: Range<usize>) -> SourceSpan {
    (range.start, range.end - range.start).into()
}

#[cfg(test)]
mod macros {
    #[macro_export]
    macro_rules! text {
        ($range:expr) => {
            Token::Text($range)
        };
    }

    #[macro_export]
    macro_rules! interp {
        ($tag:expr, $body:expr) => {
            Token::Interp {
                tag: $tag,
                body: $body,
                left: Modifier::None,
                right: Modifier::None,
            }
        };
        ($tag:expr, $body:expr, $left:ident, $right:ident) => {
            Token::Interp {
                tag: $tag,
                body: $body,
                left: Modifier::$left,
                right: Modifier::$right,
            }
        };
    }

    #[macro_export]
    macro_rules! stmt {
        ($tag:expr, $body:expr) => {
            Token::Stmt {
                tag: $tag,
                body: $body,
                left: Modifier::None,
                right: Modifier::None,
            }
        };
        ($tag:expr, $body:expr, $left:ident, $right:ident) => {
            Token::Stmt {
                tag: $tag,
                body: $body,
                left: Modifier::$left,
                right: Modifier::$right,
            }
        };
    }
}

#[cfg(any(test, feature = "testing"))]
#[allow(dead_code)]
mod test_utils {
    use std::collections::{BTreeMap, HashMap};

    use crate::{FunctionRegistry, RegistryError, Value, ValueType};

    /// Registry where every function is undefined; covers templates that never
    /// call functions.
    pub struct MockRegistry;

    impl FunctionRegistry for MockRegistry {
        fn call(&self, _name: &str, _args: &[Value]) -> Result<Value, RegistryError> {
            unreachable!()
        }
    }

    /// Registry exposing a small set of functions for exercising function calls
    /// and future statement logic (`if` conditions, `for` iterables, etc.).
    pub struct TestRegistry;

    impl FunctionRegistry for TestRegistry {
        fn call(&self, name: &str, args: &[Value]) -> Result<Value, RegistryError> {
            match name {
                "foo" => Ok(Value::Str("bar".to_string())),
                "same" => Ok(args[0].clone()),
                "mismatch" => Err(RegistryError::TypeMismatch {
                    expected: ValueType::Str,
                    got: args[0].value_type(),
                    arg_index: 0,
                }),
                "one_arg" => Err(RegistryError::ArgCount {
                    expected: 1,
                    got: args.len(),
                }),
                "two_arg" => Err(RegistryError::ArgCount {
                    expected: 2,
                    got: args.len(),
                }),
                _ => Err(RegistryError::Undefined {
                    name: name.to_owned(),
                }),
            }
        }
    }

    /// A variable scope shared by several test cases.
    pub fn var_scope() -> HashMap<String, Value> {
        let nested = BTreeMap::from_iter([("nested".to_string(), Value::Str("value".to_string()))]);
        let map = BTreeMap::from_iter([
            ("key".to_string(), Value::Str("value".to_owned())),
            ("nested".to_string(), Value::Map(nested)),
        ]);

        HashMap::from([
            ("str".to_string(), Value::Str("foobar".to_string())),
            ("num".to_string(), Value::Int(42)),
            ("neg".to_string(), Value::Int(-5)),
            ("yes".to_string(), Value::Bool(true)),
            ("no".to_string(), Value::Bool(false)),
            ("empty_list".to_string(), Value::List(vec![])),
            (
                "list".to_string(),
                Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]),
            ),
            ("map".to_string(), Value::Map(map)),
        ])
    }
}

#[cfg(any(test, feature = "testing"))]
pub use test_utils::*;
