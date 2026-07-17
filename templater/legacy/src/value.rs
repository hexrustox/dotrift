use serde::Deserialize;
use std::collections::BTreeMap;
use std::io;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    List(Vec<Value>),
    Map(BTreeMap<String, Value>),
}

pub enum ValueType {
    Str,
    Int,
    Bool,
    List,
    Map,
}

impl ValueType {
    pub fn type_name(&self) -> &'static str {
        match self {
            ValueType::Str => "String",
            ValueType::Int => "Int",
            ValueType::Bool => "Bool",
            ValueType::List => "List",
            ValueType::Map => "Map",
        }
    }
}

impl From<&Value> for ValueType {
    fn from(value: &Value) -> Self {
        match value {
            Value::Str(_) => ValueType::Str,
            Value::Int(_) => ValueType::Int,
            Value::Bool(_) => ValueType::Bool,
            Value::List(_) => ValueType::List,
            Value::Map(_) => ValueType::Map,
        }
    }
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        ValueType::from(self).type_name()
    }

    pub fn write_to<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        match self {
            Value::Str(s) => writer.write_all(s.as_bytes()),
            Value::Int(n) => write!(writer, "{n}"),
            Value::Bool(b) => write!(writer, "{b}"),
            Value::List(items) => {
                write!(writer, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(writer, ", ")?;
                    }
                    item.write_to(writer)?;
                }
                write!(writer, "]")
            }
            Value::Map(map) => {
                write!(writer, "{{")?;
                let mut first = true;
                for (key, val) in map {
                    if !first {
                        write!(writer, ", ")?;
                    }
                    first = false;
                    write!(writer, "{key}: ")?;
                    val.write_to(writer)?;
                }
                write!(writer, "}}")
            }
        }
    }
}

impl TryFrom<toml::Value> for Value {
    type Error = ();

    fn try_from(v: toml::Value) -> std::result::Result<Self, Self::Error> {
        match v {
            toml::Value::String(s) => Ok(Value::Str(s)),
            toml::Value::Integer(i) => Ok(Value::Int(i)),
            toml::Value::Boolean(b) => Ok(Value::Bool(b)),
            toml::Value::Array(arr) => {
                let items: std::result::Result<Vec<_>, _> =
                    arr.into_iter().map(Value::try_from).collect();
                Ok(Value::List(items?))
            }
            toml::Value::Table(table) => {
                let map: std::result::Result<BTreeMap<_, _>, _> = table
                    .into_iter()
                    .map(|(k, v)| Ok((k, Value::try_from(v)?)))
                    .collect();
                Ok(Value::Map(map?))
            }
            _ => Err(()),
        }
    }
}
