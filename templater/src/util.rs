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

/// Interprets `bytes` as UTF-8 without checking.
///
/// # Safety
/// Callers must only pass byte slices that are valid UTF-8.  Templater uses
/// this for identifiers, keywords, and field names that the parser has
/// already restricted to ASCII, so invalid UTF-8 is impossible here.
pub(crate) unsafe fn ascii_str_unchecked(bytes: &[u8]) -> &str {
    // SAFETY: caller guarantees `bytes` is valid UTF-8.  All call sites pass
    // parser-restricted ASCII identifiers/keywords/field names.
    unsafe { std::str::from_utf8_unchecked(bytes) }
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

    use crate::{FunctionRegistry, RegistryError, Value, value::ValueType};

    /// Re-export of the parser's reserved keyword list so integration tests
    /// can filter generated identifiers against the same source of truth the
    /// parser enforces. See `parser::RESERVED_KEYWORDS`.
    pub use crate::parser::RESERVED_KEYWORDS;

    /// Registry where every function is undefined.
    ///
    /// Use this for templates that are expected never to call functions; any
    /// call made through this registry will panic.
    pub struct MockRegistry;

    impl FunctionRegistry for MockRegistry {
        fn call(&self, _name: &str, _args: &[Value]) -> Result<Value, RegistryError> {
            unreachable!()
        }
    }

    /// Registry exposing a small set of deterministic functions for exercising
    /// function calls in unit tests.
    ///
    /// Functions:
    /// - `foo()` -> `"bar"`
    /// - `same(x)` -> `x`
    /// - `mismatch(x)` -> type mismatch error (expects `Str`)
    /// - `one_arg(x)` -> argument-count error when not given exactly one arg
    /// - `two_arg(x, y)` -> argument-count error when not given exactly two args
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

    /// A sample variable scope used by several test cases.
    ///
    /// Provides values for `str`, `num`, `neg`, `yes`, `no`, `list`,
    /// `empty_list`, and `map` (with nested map `map.nested`).
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
