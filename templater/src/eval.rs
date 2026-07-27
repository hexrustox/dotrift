use std::{borrow::Cow, collections::HashMap, io};

use miette::SourceSpan;

use crate::{
    Template, Value,
    ast::{Expr, Node},
    error::{Error, FuncError, RenderError, Result},
    function::FunctionRegistry,
    util::source_span,
};

/// A binding layer in the scope stack walked by variable resolution.
pub(crate) enum Frame<'a> {
    /// The host-provided top-level variable scope (`Template::render`'s
    /// `variables` argument). Borrowed for the lifetime of the render —
    /// no clone of the host map.
    Var(&'a HashMap<String, Value>),
    /// A `for`-loop binding. `name` is the byte span of the loop-variable
    /// identifier in the source; `value` is the current iteration's owned
    /// value. The loop variable shadows any outer binding of the same name
    /// for the body; the outer binding is restored when the frame is popped
    /// at `{% end %}`.
    Loop {
        name: std::ops::Range<usize>,
        value: Value,
    },
}

/// The active scope stack: a LIFO of [`Frame`]s. Variable resolution walks
/// from innermost outward; only `for` introduces a scope, `if`/`elif`/`else`
/// do not.
pub(crate) struct Scope<'a> {
    frames: Vec<Frame<'a>>,
}

impl<'a> Scope<'a> {
    pub(crate) fn new(variables: &'a HashMap<String, Value>) -> Self {
        Self {
            frames: vec![Frame::Var(variables)],
        }
    }

    fn push_loop(&mut self, name: std::ops::Range<usize>, value: Value) {
        self.frames.push(Frame::Loop { name, value });
    }

    /// Pops the innermost Loop frame. Called once per `push_loop`; balanced
    /// push/pop pairing is the caller's responsibility, and the stack pops
    /// naturally (the bottom Var frame is never popped in well-formed use).
    fn pop_loop(&mut self) {
        self.frames.pop();
    }
}

impl Template {
    /// Renders a sequence of nodes to the writer against the active scope.
    pub(crate) fn eval_body<W: io::Write>(
        &self,
        nodes: &[Node],
        writer: &mut W,
        scope: &mut Scope<'_>,
        functions: &dyn FunctionRegistry,
    ) -> Result<()> {
        for node in nodes {
            match node {
                Node::Text(range) => writer.write_all(&self.src.as_bytes()[range.clone()])?,
                Node::Interpolate(expr) => {
                    // String literals escape-walk directly into the writer
                    // (zero allocation, byte-preserved). All other exprs are
                    // evaluated to an owned `Value` and written via
                    // `write_top`.
                    match expr {
                        Expr::StrLit { interior, .. } => {
                            write_string_literal(self.src.as_bytes(), interior.clone(), writer)?;
                        }
                        _ => {
                            let value = eval(expr, self.src.as_bytes(), scope, functions)?;
                            value.write_top(writer)?;
                        }
                    }
                }
                Node::If {
                    branches,
                    else_body,
                } => {
                    let mut taken = false;
                    for branch in branches {
                        let cond = eval(&branch.cond, self.src.as_bytes(), scope, functions)?;
                        match cond {
                            Value::Bool(true) => {
                                self.eval_body(&branch.body, writer, scope, functions)?;
                                taken = true;
                                break;
                            }
                            Value::Bool(false) => {} // skip this branch
                            other => {
                                let span = branch.cond.span();
                                return Err(Error::render(
                                    RenderError::TypeMismatch {
                                        expected: crate::ValueType::Bool,
                                        got: other.value_type(),
                                    },
                                    SourceSpan::from((span.start, span.end - span.start)),
                                ));
                            }
                        }
                    }
                    if !taken && let Some(body) = else_body {
                        self.eval_body(body, writer, scope, functions)?;
                    }
                }
                Node::For { var, iter, body } => {
                    let iterable = eval(iter, self.src.as_bytes(), scope, functions)?;
                    match iterable {
                        Value::List(items) => {
                            // Evaluate the iterable exactly once and consume
                            // the owned Vec via into_iter (zero per-iter
                            // clones). An empty iterable never pushes a Loop
                            // frame, preserving any outer binding.
                            for value in items {
                                scope.push_loop(var.clone(), value);
                                self.eval_body(body, writer, scope, functions)?;
                                scope.pop_loop();
                            }
                        }
                        other => {
                            let span = iter.span();
                            return Err(Error::render(
                                RenderError::TypeMismatch {
                                    expected: crate::ValueType::List,
                                    got: other.value_type(),
                                },
                                SourceSpan::from((span.start, span.end - span.start)),
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Looks up `name` against the active scope stack, walking from innermost
/// outward. Returns a borrowed `Cow` for Var/Loop frames (no clone until the
/// caller asks for `into_owned`).
fn lookup<'v>(name: &str, src: &[u8], scope: &'v Scope<'v>) -> Option<Cow<'v, Value>> {
    for frame in scope.frames.iter().rev() {
        match frame {
            Frame::Var(map) => {
                if let Some(v) = map.get(name) {
                    return Some(Cow::Borrowed(v));
                }
            }
            Frame::Loop {
                name: var_range,
                value,
            } => {
                if &src[var_range.clone()] == name.as_bytes() {
                    return Some(Cow::Borrowed(value));
                }
            }
        }
    }
    None
}

/// Evaluates one expression to an owned `Value`. Per decision E1 the lookup
/// path clones the underlying value out of the borrowed scope (`StrLit` at
/// top-level interpolation is handled separately in `eval_body` for the
/// zero-allocation fast path, but nested string literals evaluate here).
fn eval(
    expr: &Expr,
    src: &[u8],
    scope: &Scope<'_>,
    functions: &dyn FunctionRegistry,
) -> Result<Value> {
    Ok(match expr {
        Expr::IntLit { value: n, .. } => Value::Int(*n),
        Expr::BoolLit { value: b, .. } => Value::Bool(*b),
        Expr::StrLit { interior, .. } => {
            let mut out = Vec::new();
            write_string_literal(src, interior.clone(), &mut out)?;
            Value::Str(String::from_utf8(out).expect("decoded bytes are valid UTF-8"))
        }
        Expr::Var(range) => {
            let name_bytes = &src[range.clone()];
            // Variable names are restricted to `[A-Za-z_][A-Za-z0-9_]*` by
            // the parser, so the byte slice is ASCII (valid UTF-8).
            let name = std::str::from_utf8(name_bytes).expect("identifier is ascii");
            match lookup(name, src, scope) {
                Some(v) => v.into_owned(),
                None => {
                    return Err(Error::render(
                        RenderError::UndefinedVariable,
                        SourceSpan::from((range.start, range.end - range.start)),
                    ));
                }
            }
        }
        Expr::List { elements, .. } => {
            let mut values = Vec::with_capacity(elements.len());
            for element in elements {
                values.push(eval(element, src, scope, functions)?);
            }
            Value::List(values)
        }
        Expr::FnCall {
            name,
            args,
            paren: paren_span,
        } => {
            let name_str = std::str::from_utf8(&src[name.clone()]).expect("identifier is ascii");
            let mut values = Vec::with_capacity(args.len());
            for arg in args {
                values.push(eval(arg, src, scope, functions)?);
            }

            match functions.call(name_str, &values) {
                Ok(value) => value,
                Err(err) => {
                    let span = func_error_span(&err, args, name, paren_span);
                    return Err(Error::func(err, span));
                }
            }
        }
        Expr::Dot { left, field } => {
            let receiver = eval(left, src, scope, functions)?;
            match &receiver {
                Value::Map(map) => {
                    let key = std::str::from_utf8(&src[field.clone()])
                        .expect("field identifier is ascii");
                    match map.get(key) {
                        Some(v) => v.clone(),
                        None => {
                            return Err(Error::render(
                                RenderError::MapKeyNotFound {
                                    key: key.to_owned(),
                                },
                                SourceSpan::from((field.start, field.end - field.start)),
                            ));
                        }
                    }
                }
                other => {
                    return Err(Error::render(
                        RenderError::TypeMismatch {
                            expected: crate::ValueType::Map,
                            got: other.value_type(),
                        },
                        SourceSpan::from((field.start, field.end - field.start)),
                    ));
                }
            }
        }
        Expr::Index {
            left,
            idx,
            idx_span,
        } => {
            let receiver = eval(left, src, scope, functions)?;
            let span = SourceSpan::from((idx_span.start, idx_span.end - idx_span.start));
            if *idx < 0 {
                return Err(Error::render(
                    RenderError::NegativeListIndex { idx: *idx },
                    span,
                ));
            }
            match receiver {
                Value::List(list) => {
                    let index = *idx as usize;
                    if index >= list.len() {
                        return Err(Error::render(
                            RenderError::ListIndexOutOfBounds {
                                idx: *idx,
                                len: list.len(),
                            },
                            span,
                        ));
                    }
                    list[index].clone()
                }
                other => {
                    return Err(Error::render(
                        RenderError::TypeMismatch {
                            expected: crate::ValueType::List,
                            got: other.value_type(),
                        },
                        span,
                    ));
                }
            }
        }
    })
}

/// Computes the source span for a function error per the spec:
/// - `Undefined`: the function name.
/// - `ArgCount`: all arguments (first arg start → last arg end); for zero
///   args, the empty parenthesized argument list `()`.
/// - `TypeMismatch`: the offending argument.
fn func_error_span(
    err: &FuncError,
    args: &[Expr],
    name: &std::ops::Range<usize>,
    paren_span: &std::ops::Range<usize>,
) -> SourceSpan {
    match err {
        FuncError::Undefined { .. } => source_span(name.clone()),
        FuncError::ArgCount { .. } => {
            if args.is_empty() {
                source_span(paren_span.clone())
            } else {
                source_span(args.first().unwrap().span().start..args.last().unwrap().span().end)
            }
        }
        FuncError::TypeMismatch { arg_index, .. } => {
            if let Some(arg) = args.get(*arg_index) {
                source_span(arg.span())
            } else {
                // unreachable
                source_span(name.clone())
            }
        }
    }
}

/// Walks the interior of a `"..."` literal (the byte range between the
/// opening and closing quotes, exclusive) and writes the decoded result
/// directly into `writer`:
///
/// - `\"` → `"`, `\\` → `\`.
/// - Any other `\X` → both bytes verbatim (no interpretation).
/// - Raw newlines and other bytes pass through unchanged, byte-for-byte
///   (no `char`-cast — non-ASCII bytes such as those inside `{{ "café" }}`
///   survive intact even though they aren't valid standalone UTF-8).
///
/// The range is guaranteed to be the interior of a *closed* string literal
/// (the parser rejects unclosed strings at parse time), so the loop is
/// infallible.
fn write_string_literal<W: io::Write>(
    src: &[u8],
    interior: std::ops::Range<usize>,
    writer: &mut W,
) -> io::Result<()> {
    let bytes = &src[interior];
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' && i + 1 < bytes.len() {
            let next = bytes[i + 1];
            match next {
                b'"' => {
                    writer.write_all(b"\"")?;
                    i += 2;
                    continue;
                }
                b'\\' => {
                    writer.write_all(b"\\")?;
                    i += 2;
                    continue;
                }
                _ => {
                    // Pass both bytes through verbatim.
                    writer.write_all(&bytes[i..i + 2])?;
                    i += 2;
                    continue;
                }
            }
        }
        writer.write_all(&bytes[i..i + 1])?;
        i += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use crate::{
        ValueType,
        parser::parse,
        scanner::scan,
        util::{TestRegistry, var_scope},
    };

    use super::*;

    fn eval_err(src: &[u8]) -> Error {
        let Node::Interpolate(expr) = parse(scan(src).unwrap(), src).unwrap().pop().unwrap() else {
            panic!("expected Interpolate")
        };
        let vars = var_scope();
        let scope = Scope::new(&vars);
        eval(&expr, src, &scope, &TestRegistry).unwrap_err()
    }

    #[test_case(br#"a\"b"# => "a\"b"; "escaped_double_quote")]
    #[test_case(br#"a\\b"# => "a\\b"; "escaped_backslash")]
    #[test_case(br#"a\nb"# => "a\\nb"; "other_backslash_verbatim")]
    #[test_case(br#"\t"# => "\\t"; "backslash_t_verbatim")]
    #[test_case(b"hello" => "hello"; "plain_text_round_trips")]
    #[test_case(b"a\nb" => "a\nb"; "raw_newline_preserved")]
    #[test_case(br#"{{ }} {# #}"# => r#"{{ }} {# #}"#; "delimiters_are_literal")]
    #[test_case(b"a}b" => "a}b"; "closing_brace_literal")]
    #[test_case(b"abc\\" => "abc\\"; "trailing_lone_backslash")]
    #[test_case(b"" => ""; "empty_interior")]
    #[test_case(b"caf\xc3\xa9" => String::from_utf8_lossy(b"caf\xc3\xa9"); "non_ascii_bytes_survive")]
    #[test_case(br#"\\\""# => "\\\""; "escaped_backslash_then_quote")]
    fn write_string_literal_round_trips(src: &[u8]) -> String {
        let mut out = Vec::new();
        write_string_literal(src, 0..src.len(), &mut out).unwrap();
        String::from_utf8_lossy(&out).to_string()
    }

    #[test_case(b"{{ 42 }}" => Value::Int(42) ; "int_pos")]
    #[test_case(b"{{ -7 }}" => Value::Int(-7) ; "int_neg")]
    #[test_case(b"{{ true }}" => Value::Bool(true) ; "bool_true")]
    #[test_case(b"{{ false }}" => Value::Bool(false) ; "bool_false")]
    #[test_case(b"{{ \"x\" }}" => Value::Str("x".to_string()) ; "str_basic")]
    #[test_case(br#"{{ "a\"b\\c" }}"# => Value::Str("a\"b\\c".to_string()) ; "str_escapes")]
    #[test_case(b"{{ \"a\nb\" }}" => Value::Str("a\nb".to_string()) ; "str_raw_newline")]
    #[test_case(b"{{ [] }}" => Value::List(vec![]) ; "list_empty")]
    #[test_case(b"{{ [1, \"x\", true] }}" => Value::List(vec![Value::Int(1), Value::Str("x".to_string()), Value::Bool(true)]) ; "list_heterogeneous")]
    #[test_case(b"{{ [[1, 2], []] }}" => Value::List(vec![Value::List(vec![Value::Int(1), Value::Int(2)]), Value::List(vec![])]) ; "list_nested")]
    #[test_case(b"{{ str }}" => Value::Str("foobar".to_string()) ; "var_str")]
    #[test_case(b"{{ num }}" => Value::Int(42) ; "var_int")]
    #[test_case(b"{{ yes }}" => Value::Bool(true) ; "var_bool")]
    #[test_case(b"{{ list }}" => Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]) ; "var_list")]
    #[test_case(b"{{ map.key }}" => Value::Str("value".to_string()) ; "dot_map_key")]
    #[test_case(b"{{ map.nested.nested }}" => Value::Str("value".to_string()) ; "dot_nested")]
    #[test_case(b"{{ list.0 }}" => Value::Int(1) ; "index_first")]
    #[test_case(b"{{ list.2 }}" => Value::Int(3) ; "index_last")]
    #[test_case(b"{{ same([10, 20]).1 }}" => Value::Int(20) ; "index_on_call")]
    #[test_case(b"{{ same(\"z\") }}" => Value::Str("z".to_string()) ; "call_same")]
    #[test_case(b"{{ foo(\"a\", \"b\") }}" => Value::Str("bar".to_string()) ; "call_foo")]
    #[test_case(b"{{ foo() }}" => Value::Str("bar".to_string()) ; "call_zero_args")]
    #[test_case(b"{{ same(same(\"deep\")) }}" => Value::Str("deep".to_string()) ; "call_nested")]
    fn eval_success(src: &[u8]) -> Value {
        let Node::Interpolate(expr) = parse(scan(src).unwrap(), src).unwrap().pop().unwrap() else {
            panic!("expected Interpolate")
        };
        let vars = var_scope();
        let scope = Scope::new(&vars);
        eval(&expr, src, &scope, &TestRegistry).unwrap()
    }

    #[test_case(b"{{ missing }}" => (RenderError::UndefinedVariable, (3, 7)) ; "undefined_variable")]
    #[test_case(b"{{ map.nope }}" => (RenderError::MapKeyNotFound { key: "nope".into() }, (7, 4)) ; "map_key_not_found")]
    #[test_case(b"{{ str.field }}" => (RenderError::TypeMismatch { expected: ValueType::Map, got: ValueType::Str }, (7, 5)) ; "map_access_on_str")]
    #[test_case(b"{{ num.field }}" => (RenderError::TypeMismatch { expected: ValueType::Map, got: ValueType::Int }, (7, 5)) ; "map_access_on_int")]
    #[test_case(b"{{ yes.field }}" => (RenderError::TypeMismatch { expected: ValueType::Map, got: ValueType::Bool }, (7, 5)) ; "map_access_on_bool")]
    #[test_case(b"{{ list.field }}" => (RenderError::TypeMismatch { expected: ValueType::Map, got: ValueType::List }, (8, 5)) ; "map_access_on_list")]
    #[test_case(b"{{ \"s\".0 }}" => (RenderError::TypeMismatch { expected: ValueType::List, got: ValueType::Str }, (7, 1)) ; "list_access_on_str")]
    #[test_case(b"{{ yes.0 }}" => (RenderError::TypeMismatch { expected: ValueType::List, got: ValueType::Bool }, (7, 1)); "index_on_bool")]
    #[test_case(b"{{ num.0 }}" => (RenderError::TypeMismatch { expected: ValueType::List, got: ValueType::Int }, (7, 1)); "index_on_int")]
    #[test_case(b"{{ map.0 }}" => (RenderError::TypeMismatch { expected: ValueType::List, got: ValueType::Map }, (7, 1)) ; "list_access_on_map")]
    #[test_case(b"{{ list.3 }}" => (RenderError::ListIndexOutOfBounds { idx: 3, len: 3 }, (8, 1)) ; "index_out_of_bounds")]
    #[test_case(b"{{ list.-1 }}" => (RenderError::NegativeListIndex { idx: -1 }, (8, 2)) ; "negative_index")]
    fn eval_render(src: &[u8]) -> (RenderError, (usize, usize)) {
        match eval_err(src) {
            Error::Render { err, span, .. } => (err, (span.offset(), span.len())),
            e => panic!("expected Render error, got {e:?}"),
        }
    }

    #[test_case(b"{{ nope() }}" => (FuncError::Undefined { name: "nope".into() }, (3, 4)) ; "undefined_function")]
    #[test_case(b"{{ same() }}" => (FuncError::ArgCount { expected: 1, got: 0 }, (7, 2)) ; "arg_count_zero_args")]
    #[test_case(b"{{ exact1(\"a\", \"b\") }}" => (FuncError::ArgCount { expected: 1, got: 2 }, (10, 8)) ; "arg_count_multi_args")]
    #[test_case(b"{{ exact1(12) }}" => (FuncError::TypeMismatch { expected: ValueType::Str, got: ValueType::Int, arg_index: 0 }, (10, 2)) ; "type_mismatch_arg")]
    fn eval_func(src: &[u8]) -> (FuncError, (usize, usize)) {
        match eval_err(src) {
            Error::Func { err, span, .. } => (err, (span.offset(), span.len())),
            e => panic!("expected Func error, got {e:?}"),
        }
    }

    #[test_case(b"{% if yes %}Y{% end %}" => "Y"; "if_true")]
    #[test_case(b"{% if no %}Y{% else %}N{% end %}" => "N"; "if_false")]
    #[test_case(b"{% if no %}A{% elif yes %}B{% else %}C{% end %}" => "B"; "if_elif")]
    #[test_case(b"{% if yes %}{% if yes %}ok{% end %}{% end %}" => "ok"; "nested_if")]
    #[test_case(b"{% for x in list %}{{x}},{% end %}" => "1,2,3,"; "for_list_var")]
    #[test_case(b"{% for x in [10, 20] %}{{x}};{% end %}" => "10;20;"; "for_list_literal")]
    #[test_case(b"{% for x in same([\"a\", \"b\"]) %}{{x}}{% end %}" => "ab"; "for_fn_call")]
    #[test_case(b"{% for x in list %}{{ str }}{% end %}" => "foobarfoobarfoobar"; "for_uses_outer_var")]
    #[test_case(b"{% for str in list %}{{str}}{% end %}" => "123"; "for_shadows_outer_var")]
    #[test_case(b"{% for str in list %}{{str}}{% end %}{{str}}" => "123foobar"; "for_restores_after_end")]
    #[test_case(b"{% for x in empty_list %}{{x}}{% end %}after" => "after"; "for_empty_iterable_silent")]
    #[test_case(b"{% if false %}{% for x in [1] %}{{ missing }}{% end %}{% end %}" => ""; "for_in_untaken_branch_silent")]
    fn eval_body_if(src: &[u8]) -> String {
        let template = Template {
            nodes: parse(scan(src).unwrap(), src).unwrap(),
            src: crate::Source::Owned(src.to_vec()),
        };
        let mut out = Vec::new();
        let vars = var_scope();
        let mut scope = Scope::new(&vars);
        template
            .eval_body(&template.nodes, &mut out, &mut scope, &TestRegistry)
            .unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test_case(b"{% if str %}x{% end %}" => (RenderError::TypeMismatch { expected: ValueType::Bool, got: ValueType::Str }, (6, 3)); "if_cond_str")]
    #[test_case(b"{% if no %}{% elif num %}x{% end %}" => (RenderError::TypeMismatch { expected: ValueType::Bool, got: ValueType::Int }, (19, 3)); "elif_cond_int")]
    #[test_case(b"{% for x in str %}{{x}}{% end %}" => (RenderError::TypeMismatch { expected: ValueType::List, got: ValueType::Str }, (12, 3)); "for_iterable_str")]
    fn eval_body_if_error(src: &[u8]) -> (RenderError, (usize, usize)) {
        let template = Template {
            nodes: parse(scan(src).unwrap(), src).unwrap(),
            src: crate::Source::Owned(src.to_vec()),
        };
        let mut out = Vec::new();
        let vars = var_scope();
        let mut scope = Scope::new(&vars);
        match template
            .eval_body(&template.nodes, &mut out, &mut scope, &TestRegistry)
            .unwrap_err()
        {
            Error::Render { err, span, .. } => (err, (span.offset(), span.len())),
            e => panic!("expected Render error, got {e:?}"),
        }
    }
}
