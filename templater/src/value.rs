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
    /// Renders the value at the top level of an interpolation (the
    /// "canonical string form" of the spec for Str/Int/Bool only in this
    /// slice — List/Map are not reachable yet).
    ///
    /// - **Str** — verbatim (no escape processing, no delimiter escaping).
    /// - **Int** — decimal, with a leading `-` for negatives; `{}` formatting
    ///   of `i64` already does this.
    /// - **Bool** — `true` / `false`.
    pub(crate) fn write_top<W: io::Write>(&self, writer: &mut W) -> io::Result<()> {
        match self {
            Value::Str(s) => writer.write_all(s.as_bytes()),
            Value::Int(n) => write!(writer, "{n}"),
            Value::Bool(b) => write!(writer, "{b}"),
            // Not reachable in this slice — no list/map literals or function
            // calls exist yet. Calling `write_top` on a List/Map is a
            // programmer error in the host, not a templater error.
            Value::List(_) | Value::Map(_) => unreachable!(
                "write_top on List/Map is unreachable until list literals (ticket 05) \
                 and function calls (ticket 04) ship"
            ),
        }
    }
}
