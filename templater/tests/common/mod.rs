use std::collections::{BTreeMap, HashMap};

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

/// A variable scope containing aggregate values (lists and maps).
#[allow(dead_code)]
pub fn aggregate_scope() -> HashMap<String, Value> {
    let mut user = BTreeMap::new();
    user.insert("name".to_string(), Value::Str("ada".to_string()));
    user.insert("age".to_string(), Value::Int(42));

    let mut prefs = BTreeMap::new();
    prefs.insert("theme".to_string(), Value::Str("dark".to_string()));
    user.insert("prefs".to_string(), Value::Map(prefs));

    HashMap::from([
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
