use std::collections::BTreeMap;

/// A runtime value produced by evaluating an expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    List(Vec<Value>),
    Map(BTreeMap<String, Value>),
}

/// The type of a [`Value`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    Str,
    Int,
    Bool,
    List,
    Map,
}
