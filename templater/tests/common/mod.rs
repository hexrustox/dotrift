use std::collections::{BTreeMap, HashMap};

use templater::{FuncError, FunctionRegistry, Value, ValueType};

/// Registry where every function is undefined; covers templates that never
/// call functions.
#[allow(dead_code)]
pub struct MockRegistry;

impl FunctionRegistry for MockRegistry {
    fn call(&self, name: &str, _args: &[Value]) -> Result<Value, FuncError> {
        Err(FuncError::Undefined {
            name: name.to_owned(),
        })
    }
}

/// Registry exposing a small set of functions for exercising function calls
/// and future statement logic (`if` conditions, `for` iterables, etc.).
#[allow(dead_code)]
pub struct TestRegistry;

impl FunctionRegistry for TestRegistry {
    fn call(&self, name: &str, args: &[Value]) -> Result<Value, FuncError> {
        match name {
            "eq" => {
                if args.len() != 2 {
                    return Err(FuncError::ArgCount {
                        expected: 2,
                        got: args.len(),
                    });
                }
                Ok(Value::Bool(args[0] == args[1]))
            }
            "not" => {
                if args.len() != 1 {
                    return Err(FuncError::ArgCount {
                        expected: 1,
                        got: args.len(),
                    });
                }
                match &args[0] {
                    Value::Bool(b) => Ok(Value::Bool(!b)),
                    other => Err(FuncError::TypeMismatch {
                        expected: ValueType::Bool,
                        got: other.value_type(),
                        arg_index: 0,
                    }),
                }
            }
            "length" => {
                if args.len() != 1 {
                    return Err(FuncError::ArgCount {
                        expected: 1,
                        got: args.len(),
                    });
                }
                match &args[0] {
                    Value::List(items) => Ok(Value::Int(items.len() as i64)),
                    other => Err(FuncError::TypeMismatch {
                        expected: ValueType::List,
                        got: other.value_type(),
                        arg_index: 0,
                    }),
                }
            }
            "join" => {
                if args.is_empty() {
                    return Err(FuncError::ArgCount {
                        expected: 1,
                        got: 0,
                    });
                }
                let sep = match &args[0] {
                    Value::Str(s) => s.clone(),
                    other => {
                        return Err(FuncError::TypeMismatch {
                            expected: ValueType::Str,
                            got: other.value_type(),
                            arg_index: 0,
                        });
                    }
                };
                let rendered: Vec<String> = args[1..].iter().map(Value::render).collect();
                Ok(Value::Str(rendered.join(&sep)))
            }
            _ => Err(FuncError::Undefined {
                name: name.to_owned(),
            }),
        }
    }
}

/// A variable scope shared by several test cases.
#[allow(dead_code)]
pub fn var_scope() -> HashMap<String, Value> {
    let mut user = BTreeMap::new();
    user.insert("name".to_string(), Value::Str("ada".to_string()));
    user.insert("age".to_string(), Value::Int(42));

    let mut prefs = BTreeMap::new();
    prefs.insert("theme".to_string(), Value::Str("dark".to_string()));
    user.insert("prefs".to_string(), Value::Map(prefs));

    HashMap::from([
        ("name".to_string(), Value::Str("world".to_string())),
        ("count".to_string(), Value::Int(42)),
        ("neg".to_string(), Value::Int(-5)),
        ("flag".to_string(), Value::Bool(true)),
        ("off".to_string(), Value::Bool(false)),
        ("empty".to_string(), Value::List(vec![])),
        (
            "items".to_string(),
            Value::List(vec![
                Value::Str("a".to_string()),
                Value::Str("b".to_string()),
                Value::Int(3),
            ]),
        ),
        (
            "nested".to_string(),
            Value::List(vec![
                Value::List(vec![Value::Int(1), Value::Int(2)]),
                Value::List(vec![Value::Int(3), Value::Int(4)]),
            ]),
        ),
        ("user".to_string(), Value::Map(user)),
    ])
}
