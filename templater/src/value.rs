use std::collections::BTreeMap;
use std::io;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    List(Vec<Value>),
    Map(BTreeMap<String, Value>),
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Str(_) => "String",
            Value::Int(_) => "Int",
            Value::Bool(_) => "Bool",
            Value::List(_) => "List",
            Value::Map(_) => "Map",
        }
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
