use std::collections::BTreeMap;

use std::io;

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

impl std::fmt::Display for ValueType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Str => "string",
                Self::Int => "integer",
                Self::Bool => "boolean",
                Self::List => "list",
                Self::Map => "map",
            }
        )
    }
}

impl Value {
    /// Renders this value using its top-level interpolation form.
    pub fn render(&self) -> String {
        let mut buf = Vec::new();
        self.write_top(&mut buf)
            .expect("writing to an in-memory buffer never fails");
        // write_top only emits valid UTF-8 for Str; non-Str are ASCII.
        String::from_utf8(buf).expect("rendered value is valid UTF-8")
    }

    /// The runtime type of this value.
    pub fn value_type(&self) -> ValueType {
        match self {
            Value::Str(_) => ValueType::Str,
            Value::Int(_) => ValueType::Int,
            Value::Bool(_) => ValueType::Bool,
            Value::List(_) => ValueType::List,
            Value::Map(_) => ValueType::Map,
        }
    }

    /// Renders the value at the top level of an interpolation.
    ///
    /// - **Str** — verbatim (no escape processing, no delimiter escaping).
    /// - **Int** — decimal, with a leading `-` for negatives.
    /// - **Bool** — `true` / `false`.
    /// - **List/Map** — their canonical nested forms via [`Value::write_nested`].
    pub(crate) fn write_top<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        match self {
            Value::Str(s) => writer.write_all(s.as_bytes()),
            Value::Int(n) => write!(writer, "{n}"),
            Value::Bool(b) => write!(writer, "{b}"),
            Value::List(_) | Value::Map(_) => self.write_nested(writer),
        }
    }

    /// Renders the canonical nested form for List and Map values.
    ///
    /// - List: `[` comma-separated canonical forms `]`; empty `[]`.
    /// - Map: `{` comma-separated `"key": value` pairs `}`; empty `{}`.
    /// - String elements and String keys are double-quoted, escaping only `\`
    ///   and `"` byte-by-byte; other backslash sequences pass through.
    /// - String values inside aggregates are not re-escaped for delimiters.
    /// - Map iteration follows `BTreeMap`'s natural order.
    pub(crate) fn write_nested<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        match self {
            Value::List(items) => {
                writer.write_all(b"[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        writer.write_all(b", ")?;
                    }
                    item.write_nested(writer)?;
                }
                writer.write_all(b"]")?;
            }
            Value::Map(map) => {
                writer.write_all(b"{")?;
                for (i, (key, value)) in map.iter().enumerate() {
                    if i > 0 {
                        writer.write_all(b", ")?;
                    }
                    writer.write_all(b"\"")?;
                    write_escaped_string(writer, key.as_bytes())?;
                    writer.write_all(b"\": ")?;
                    value.write_nested(writer)?;
                }
                writer.write_all(b"}")?;
            }
            Value::Str(s) => {
                writer.write_all(b"\"")?;
                write_escaped_string(writer, s.as_bytes())?;
                writer.write_all(b"\"")?;
            }
            Value::Int(n) => write!(writer, "{n}")?,
            Value::Bool(b) => write!(writer, "{b}")?,
        }
        Ok(())
    }
}

/// Writes bytes with only `\"` and `\\` escape processing.
fn write_escaped_string<W: io::Write>(writer: &mut W, bytes: &[u8]) -> io::Result<()> {
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\\' => writer.write_all(br"\\"),
            b'"' => writer.write_all(b"\\\""),
            _ => writer.write_all(&bytes[i..i + 1]),
        }?;
        i += 1;
    }
    Ok(())
}
