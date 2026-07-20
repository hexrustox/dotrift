mod common;

use std::collections::{BTreeMap, HashMap};

use common::MockRegistry;
use proptest::prelude::*;
use templater::Value;

/// All six delimiter sequences recognized by the scanner.
const DELIMS: &[&[u8]] = &[b"{{", b"}}", b"{%", b"%}", b"{#", b"#}"];

/// Bytes that are safe for a plain-text segment: they cannot form or hide a
/// delimiter, nor can they be interpreted as an escape prefix.
fn safe_plain_bytes() -> Vec<u8> {
    // Printable ASCII except `{` `}` `%` `#` `\`.
    (32u8..=126)
        .filter(|&b| !matches!(b, b'{' | b'}' | b'%' | b'#' | b'\\'))
        .collect()
}

/// A generated interpolation value and its expected rendered output.
#[derive(Debug, Clone)]
enum InterpValue {
    /// String literal `"..."`.
    String(Vec<u8>),
    /// Integer literal.
    Int(i64),
    /// Boolean literal.
    Bool(bool),
    /// List literal `[...]`.
    List(Vec<InterpValue>),
    /// Variable reference named `name` with runtime value `value`.
    Variable { name: String, value: Value },
}

impl InterpValue {
    /// Source expression bytes and expected rendered output for this value.
    ///
    /// Variable references are resolved from the supplied `variables` map so
    /// that duplicate variable names share the same value, matching how the
    /// engine resolves names at render time.
    fn parts(&self, variables: &HashMap<String, Value>) -> (Vec<u8>, Vec<u8>) {
        match self {
            InterpValue::String(bytes) => {
                let mut src = Vec::with_capacity(bytes.len() + 2);
                src.push(b'\"');
                for &b in bytes {
                    match b {
                        b'\\' => src.extend_from_slice(b"\\\\"),
                        b'\"' => src.extend_from_slice(b"\\\""),
                        _ => src.push(b),
                    }
                }
                src.push(b'\"');
                (src, bytes.clone())
            }
            InterpValue::Int(n) => {
                let rendered = n.to_string().into_bytes();
                (rendered.clone(), rendered)
            }
            InterpValue::Bool(b) => {
                let rendered = if *b {
                    b"true".to_vec()
                } else {
                    b"false".to_vec()
                };
                (rendered.clone(), rendered)
            }
            InterpValue::List(elements) => {
                let mut src = Vec::new();
                src.push(b'[');
                for (i, elem) in elements.iter().enumerate() {
                    if i > 0 {
                        src.extend_from_slice(b", ");
                    }
                    let (elem_src, _) = elem.parts(variables);
                    src.extend(elem_src);
                }
                src.push(b']');
                let value = self.to_runtime_value(variables);
                let rendered = render_value_nested(&value);
                (src, rendered)
            }
            InterpValue::Variable { name, .. } => {
                let value = variables
                    .get(name)
                    .unwrap_or_else(|| panic!("generated variable {name} must be bound"));
                let rendered = render_value_top(value);
                (name.clone().into_bytes(), rendered)
            }
        }
    }

    /// Resolve this expression to the runtime `Value` it would evaluate to.
    ///
    /// Variable references are looked up from the supplied map; list literals
    /// are resolved recursively.
    fn to_runtime_value(&self, variables: &HashMap<String, Value>) -> Value {
        match self {
            InterpValue::String(bytes) => {
                Value::Str(String::from_utf8(bytes.clone()).expect("ASCII string"))
            }
            InterpValue::Int(n) => Value::Int(*n),
            InterpValue::Bool(b) => Value::Bool(*b),
            InterpValue::List(elements) => Value::List(
                elements
                    .iter()
                    .map(|e| e.to_runtime_value(variables))
                    .collect(),
            ),
            InterpValue::Variable { name, .. } => variables
                .get(name)
                .cloned()
                .unwrap_or_else(|| panic!("generated variable {name} must be bound")),
        }
    }

    /// Add every variable referenced by this expression to `bindings`.
    fn collect_bindings(&self, bindings: &mut HashMap<String, Value>) {
        match self {
            InterpValue::Variable { name, value } => {
                bindings.insert(name.clone(), value.clone());
            }
            InterpValue::List(elements) => {
                for elem in elements {
                    elem.collect_bindings(bindings);
                }
            }
            _ => {}
        }
    }
}

/// Top-level interpolation output for a `Value`.
fn render_value_top(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    match value {
        Value::Str(s) => out.extend_from_slice(s.as_bytes()),
        Value::Int(n) => out.extend(n.to_string().into_bytes()),
        Value::Bool(b) => out.extend((if *b { "true" } else { "false" }).bytes()),
        Value::List(_) | Value::Map(_) => render_value_nested_into(value, &mut out),
    }
    out
}

/// Canonical nested form for a `Value` (used inside lists/maps and for
/// top-level aggregates, which have the same canonical representation).
fn render_value_nested(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    render_value_nested_into(value, &mut out);
    out
}

fn render_value_nested_into(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::List(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.extend_from_slice(b", ");
                }
                render_value_nested_into(item, out);
            }
            out.push(b']');
        }
        Value::Map(map) => {
            out.push(b'{');
            for (i, (key, value)) in map.iter().enumerate() {
                if i > 0 {
                    out.extend_from_slice(b", ");
                }
                out.push(b'\"');
                write_escaped_string(key.as_bytes(), out);
                out.extend_from_slice(b"\": ");
                render_value_nested_into(value, out);
            }
            out.push(b'}');
        }
        Value::Str(s) => {
            out.push(b'\"');
            write_escaped_string(s.as_bytes(), out);
            out.push(b'\"');
        }
        Value::Int(n) => out.extend(n.to_string().into_bytes()),
        Value::Bool(b) => out.extend((if *b { "true" } else { "false" }).bytes()),
    }
}

/// Escapes `"` and `\\` for nested string output.
fn write_escaped_string(bytes: &[u8], out: &mut Vec<u8>) {
    for &b in bytes {
        match b {
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\"' => out.extend_from_slice(b"\\\""),
            _ => out.push(b),
        }
    }
}

/// A generated template segment. It carries both the source bytes it
/// contributes and the expected rendered output for those bytes.
#[derive(Debug, Clone)]
enum Segment {
    /// Plain text containing no delimiter or backslash bytes.
    Plain(Vec<u8>),
    /// An odd run of backslashes followed by a delimiter. Renders as the
    /// literal delimiter bytes.
    Escaped {
        backslashes: usize,
        delim: &'static [u8],
    },
    /// A `{{ ... }}` interpolation expression.
    Interp(InterpValue),
    /// A `{# ... #}` comment; produces no output.
    Comment(Vec<u8>),
}

impl Segment {
    fn source_and_rendered(&self, variables: &HashMap<String, Value>) -> (Vec<u8>, Vec<u8>) {
        match self {
            Segment::Plain(bytes) => (bytes.clone(), bytes.clone()),
            Segment::Escaped { backslashes, delim } => {
                let mut src = Vec::new();
                src.extend(std::iter::repeat_n(b'\\', *backslashes));
                src.extend_from_slice(delim);

                let mut out = Vec::new();
                out.extend(std::iter::repeat_n(b'\\', (backslashes - 1) / 2));
                out.extend_from_slice(delim);
                (src, out)
            }
            Segment::Interp(value) => {
                let (expr, rendered) = value.parts(variables);
                let mut src = Vec::new();
                src.extend_from_slice(b"{{ ");
                src.extend(expr);
                src.extend_from_slice(b" }}");
                (src, rendered)
            }
            Segment::Comment(bytes) => {
                let mut src = Vec::new();
                src.extend_from_slice(b"{# ");
                src.extend(bytes);
                src.extend_from_slice(b" #}");
                (src, Vec::new())
            }
        }
    }

    /// Collect all variable bindings introduced by this segment into `map`.
    fn collect_bindings(&self, map: &mut HashMap<String, Value>) {
        if let Segment::Interp(value) = self {
            value.collect_bindings(map);
        }
    }
}

/// Letters and underscores valid in an identifier start position.
fn ident_start_bytes() -> Vec<u8> {
    (b'A'..=b'Z').chain(b'a'..=b'z').chain([b'_']).collect()
}

/// Letters, digits, and underscores valid after the first identifier byte.
fn ident_body_bytes() -> Vec<u8> {
    (b'0'..=b'9')
        .chain(b'A'..=b'Z')
        .chain(b'a'..=b'z')
        .chain([b'_'])
        .collect()
}

fn ident_strategy() -> impl Strategy<Value = String> {
    (
        prop::sample::select(ident_start_bytes()),
        prop::collection::vec(prop::sample::select(ident_body_bytes()), 0..=15),
    )
        .prop_map(|(start, rest)| {
            let mut bytes = Vec::with_capacity(1 + rest.len());
            bytes.push(start);
            bytes.extend(rest);
            String::from_utf8(bytes).expect("identifier bytes are ASCII")
        })
        .prop_filter("identifier must not be a boolean keyword", |name| {
            name != "true" && name != "false"
        })
}

/// String literal characters excluding `"`. If the generated bytes end with a
/// backslash it is dropped, so the closing quote is never escaped away.
fn string_strategy() -> impl Strategy<Value = Vec<u8>> {
    let bytes: Vec<u8> = (32u8..=126).filter(|&b| b != b'"').collect();
    prop::collection::vec(prop::sample::select(bytes), 0..=32).prop_map(|mut s| {
        if s.last() == Some(&b'\\') {
            s.pop();
        }
        s
    })
}

fn int_strategy() -> impl Strategy<Value = InterpValue> {
    any::<i64>().prop_map(InterpValue::Int)
}

fn bool_strategy() -> impl Strategy<Value = InterpValue> {
    any::<bool>().prop_map(InterpValue::Bool)
}

fn variable_strategy() -> impl Strategy<Value = InterpValue> {
    (ident_strategy(), value_strategy())
        .prop_map(|(name, value)| InterpValue::Variable { name, value })
}

fn value_strategy() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        string_strategy().prop_map(|s| Value::Str(String::from_utf8(s).expect("ASCII string"))),
        any::<i64>().prop_map(Value::Int),
        any::<bool>().prop_map(Value::Bool),
    ]
    .boxed();

    leaf.prop_recursive(
        2,  // max recursion depth
        16, // target total elements
        4,  // expected collection size
        |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..=4).prop_map(Value::List),
                prop::collection::hash_map(ident_strategy(), inner, 0..=4).prop_map(|m| {
                    Value::Map(m.into_iter().collect::<BTreeMap<String, Value>>())
                }),
            ]
        },
    )
}

fn expr_strategy() -> impl Strategy<Value = InterpValue> {
    let leaf = prop_oneof![
        string_strategy().prop_map(InterpValue::String),
        int_strategy(),
        bool_strategy(),
        variable_strategy(),
    ]
    .boxed();

    leaf.prop_recursive(
        2,  // max recursion depth
        16, // target total elements
        4,  // expected collection size
        |inner| prop::collection::vec(inner, 0..=4).prop_map(InterpValue::List),
    )
}

fn interp_strategy() -> impl Strategy<Value = Segment> {
    expr_strategy().prop_map(Segment::Interp)
}

fn plain_strategy() -> impl Strategy<Value = Segment> {
    prop::collection::vec(prop::sample::select(safe_plain_bytes()), 0..=32).prop_map(Segment::Plain)
}

fn escaped_strategy() -> impl Strategy<Value = Segment> {
    (1usize..=7, prop::sample::select(DELIMS))
        .prop_filter("odd backslash count", |(n, _)| n % 2 == 1)
        .prop_map(|(backslashes, delim)| Segment::Escaped { backslashes, delim })
}

fn comment_strategy() -> impl Strategy<Value = Segment> {
    prop::collection::vec(prop::sample::select(safe_plain_bytes()), 0..=32)
        .prop_map(Segment::Comment)
}

fn segment_strategy() -> impl Strategy<Value = Segment> {
    prop_oneof![
        plain_strategy().boxed(),
        escaped_strategy().boxed(),
        interp_strategy().boxed(),
        comment_strategy().boxed(),
    ]
}

proptest! {
    #[test]
    fn generated_templates_render_predictably(
        segments in prop::collection::vec(segment_strategy(), 0..=32)
    ) {
        let mut variables: HashMap<String, Value> = HashMap::new();
        for seg in &segments {
            seg.collect_bindings(&mut variables);
        }

        let mut source = Vec::new();
        let mut expected = Vec::new();
        for seg in &segments {
            let (src, out) = seg.source_and_rendered(&variables);
            source.extend(src);
            expected.extend(out);
        }

        let template = templater::Template::from_bytes(source.clone())
            .expect("generated template must parse");
        let mut actual = Vec::new();
        template
            .render(&mut actual, &variables, &MockRegistry)
            .expect("generated template must render");

        prop_assert_eq!(actual, expected);
    }
}
