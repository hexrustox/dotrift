use std::{borrow::Cow, collections::HashMap, io, ops::Range};

use crate::{
    Template, Value,
    ast::{Expr, Node},
    error::{Error, RegistryError, RenderError},
    function::FunctionRegistry,
    util::{ascii_str_unchecked, source_span},
    value::ValueType,
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
    Loop { name: Range<usize>, value: Value },
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

    fn push_loop(&mut self, name: Range<usize>, value: Value) {
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
    ) -> Result<(), Error> {
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
                                return Err(Error::Render(RenderError::TypeMismatch {
                                    expected: ValueType::Bool,
                                    got: other.value_type(),
                                    span: source_span(span),
                                }));
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
                            return Err(Error::Render(RenderError::TypeMismatch {
                                expected: ValueType::List,
                                got: other.value_type(),
                                span: source_span(span),
                            }));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Looks up `name` against the active scope stack, walking from innermost
/// outward. The Var frame holds borrowed variables from the host, while a Loop
/// frame owns the current iteration value; both are returned as a borrowed
/// `Cow` so the caller can cheaply take ownership with `into_owned`.
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

/// Evaluates one expression to an owned `Value`.
///
/// Variable and loop-variable lookups clone the underlying value out of the
/// borrowed scope. Top-level string interpolations in `eval_body` are handled
/// separately so the literal bytes can be written directly, but nested string
/// literals evaluate here.
fn eval(
    expr: &Expr,
    src: &[u8],
    scope: &Scope<'_>,
    functions: &dyn FunctionRegistry,
) -> Result<Value, Error> {
    Ok(match expr {
        Expr::IntLit { value: n, .. } => Value::Int(*n),
        Expr::BoolLit { value: b, .. } => Value::Bool(*b),
        Expr::StrLit { interior, .. } => {
            let mut out = Vec::new();
            write_string_literal(src, interior.clone(), &mut out)?;
            // SAFETY: the string literal decoder only emits bytes that came
            // directly from the source, which is valid UTF-8.
            Value::Str(unsafe { String::from_utf8_unchecked(out) })
        }
        Expr::Var(range) => {
            let name_bytes = &src[range.clone()];
            // SAFETY: identifiers are parser-restricted to ASCII.
            let name = unsafe { ascii_str_unchecked(name_bytes) };
            match lookup(name, src, scope) {
                Some(v) => v.into_owned(),
                None => {
                    return Err(Error::Render(RenderError::UndefinedVariable {
                        name: name.to_string(),
                        span: source_span(range.clone()),
                    }));
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
            // SAFETY: function names are parser-restricted to ASCII.
            let name_str = unsafe { ascii_str_unchecked(&src[name.clone()]) };
            let mut values = Vec::with_capacity(args.len());
            for arg in args {
                values.push(eval(arg, src, scope, functions)?);
            }

            match functions.call(name_str, &values) {
                Ok(value) => value,
                Err(err) => match err {
                    RegistryError::Custom { msg, indexes } => {
                        let spans = custom_spans(&indexes, args, name, paren_span);
                        return Err(Error::Render(RenderError::Function { msg, spans }));
                    }
                    err => {
                        let span = func_error_span(&err, args, name, paren_span);
                        return Err(Error::Render(match err {
                            RegistryError::Undefined { name: n } => {
                                RenderError::FunctionUndefined { name: n, span }
                            }
                            RegistryError::ArgCount { expected, got } => {
                                RenderError::FunctionArgCount {
                                    expected,
                                    got,
                                    span,
                                }
                            }
                            RegistryError::TypeMismatch { expected, got, .. } => {
                                RenderError::TypeMismatch {
                                    expected,
                                    got,
                                    span,
                                }
                            }
                            RegistryError::Custom { .. } => unreachable!(),
                        }));
                    }
                },
            }
        }
        Expr::Dot { left, field } => {
            let receiver = eval(left, src, scope, functions)?;
            match &receiver {
                Value::Map(map) => {
                    // SAFETY: field names are parser-restricted to ASCII.
                    let key = unsafe { ascii_str_unchecked(&src[field.clone()]) };
                    match map.get(key) {
                        Some(v) => v.clone(),
                        None => {
                            return Err(Error::Render(RenderError::MapKeyNotFound {
                                key: key.to_owned(),
                                span: source_span(field.clone()),
                            }));
                        }
                    }
                }
                other => {
                    let span = left.span();
                    return Err(Error::Render(RenderError::TypeMismatch {
                        expected: ValueType::Map,
                        got: other.value_type(),
                        span: source_span(span),
                    }));
                }
            }
        }
        Expr::Index {
            left,
            idx,
            idx_span,
        } => {
            let receiver = eval(left, src, scope, functions)?;
            let span = source_span(idx_span.clone());
            if *idx < 0 {
                return Err(Error::Render(RenderError::NegativeListIndex {
                    idx: *idx,
                    span,
                }));
            }
            match receiver {
                Value::List(list) => {
                    let index = *idx as usize;
                    if index >= list.len() {
                        return Err(Error::Render(RenderError::ListIndexOutOfBounds {
                            idx: *idx,
                            len: list.len(),
                            span,
                        }));
                    }
                    list[index].clone()
                }
                other => {
                    let span = left.span();
                    return Err(Error::Render(RenderError::TypeMismatch {
                        expected: ValueType::List,
                        got: other.value_type(),
                        span: source_span(span),
                    }));
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
/// - `Custom`: handled separately by [`custom_spans`], which flags multiple
///   arguments.
fn func_error_span(
    err: &RegistryError,
    args: &[Expr],
    name: &Range<usize>,
    paren_span: &Range<usize>,
) -> miette::SourceSpan {
    match err {
        RegistryError::Undefined { .. } => source_span(name.clone()),
        RegistryError::ArgCount { expected, got } => {
            if args.is_empty() {
                source_span(paren_span.clone())
            } else if got > expected {
                source_span(args[*expected - 1].span().end..args[*got - 1].span().end)
            } else {
                source_span(args[got - 1].span().end..args[got - 1].span().end + 1)
            }
        }
        RegistryError::TypeMismatch { arg_index, .. } => source_span(args[*arg_index].span()),
        RegistryError::Custom { .. } => unreachable!(),
    }
}

/// Computes the spans for a host-defined [`RegistryError::Custom`]: one span
/// per flagged argument index. With no indexes, falls back to the full
/// function-call span so the error always has something to underline.
fn custom_spans(
    indexes: &[usize],
    args: &[Expr],
    name: &Range<usize>,
    paren_span: &Range<usize>,
) -> Vec<miette::SourceSpan> {
    if indexes.is_empty() {
        return vec![source_span(name.start..paren_span.end)];
    }
    indexes
        .iter()
        .map(|&i| source_span(args[i].span()))
        .collect()
}

/// Walks the interior of a `"..."` literal (the byte range between the
/// opening and closing quotes, exclusive) and writes the decoded result
/// directly into `writer`:
///
/// - `\"` → `"`, `\\` → `\`.
/// - Any other `\X` → both bytes verbatim (no interpretation).
/// - Raw newlines and other bytes pass through unchanged, byte-for-byte
///   (no `char`-cast; multibyte UTF-8 sequences are preserved intact).
///
/// The range is guaranteed to be the interior of a *closed* string literal
/// (the parser rejects unclosed strings at parse time), so the loop is
/// infallible.
fn write_string_literal<W: io::Write>(
    src: &[u8],
    interior: Range<usize>,
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
        parser::parse,
        scanner::scan,
        util::{TestRegistry, var_scope},
    };

    use super::*;

    fn interp_expr(src: &[u8]) -> Expr {
        let Node::Interpolate(expr) = parse(scan(src).unwrap(), src).unwrap().pop().unwrap() else {
            panic!("expected Interpolate")
        };
        expr
    }

    fn render(src: &[u8], vars: &HashMap<String, Value>) -> std::result::Result<String, Error> {
        let mut out = Vec::new();
        Template::from_bytes(src).render(&mut out, vars, &TestRegistry)?;
        Ok(String::from_utf8(out).unwrap())
    }

    #[test_case(br#"a\"b"# => "a\"b"; "collapses_escaped_double_quote")]
    #[test_case(br#"a\\b"# => "a\\b"; "collapses_escaped_backslash")]
    #[test_case(br#"a\nb"# => "a\\nb"; "preserves_unknown_escape_verbatim")]
    #[test_case(br#"\t"# => "\\t"; "preserves_backslash_t_verbatim")]
    #[test_case(b"hello" => "hello"; "passes_plain_text_unchanged")]
    #[test_case(b"a\nb" => "a\nb"; "preserves_raw_newline")]
    #[test_case(br#"{{ }} {# #}"# => r#"{{ }} {# #}"#; "preserves_template_delimiters")]
    #[test_case(b"a}b" => "a}b"; "preserves_lone_closing_brace")]
    #[test_case(b"abc\\" => "abc\\"; "preserves_trailing_lone_backslash")]
    #[test_case(b"" => ""; "empty_interior_writes_nothing")]
    #[test_case(b"caf\xc3\xa9" => String::from_utf8_lossy(b"caf\xc3\xa9"); "preserves_non_ascii_bytes")]
    #[test_case(br#"\\\""# => "\\\""; "collapses_backslash_pair_before_quote")]
    fn decodes_string_literal_contents(src: &[u8]) -> String {
        let mut out = Vec::new();
        write_string_literal(src, 0..src.len(), &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test_case(b"{{ 42 }}" => Value::Int(42) ; "evaluates_positive_integer")]
    #[test_case(b"{{ -7 }}" => Value::Int(-7) ; "evaluates_negative_integer")]
    #[test_case(b"{{ true }}" => Value::Bool(true) ; "evaluates_true_boolean")]
    #[test_case(b"{{ false }}" => Value::Bool(false) ; "evaluates_false_boolean")]
    #[test_case(b"{{ \"x\" }}" => Value::Str("x".to_string()) ; "evaluates_plain_string")]
    #[test_case(br#"{{ "a\"b\\c" }}"# => Value::Str("a\"b\\c".to_string()) ; "evaluates_escaped_string")]
    #[test_case(b"{{ \"a\nb\" }}" => Value::Str("a\nb".to_string()) ; "evaluates_string_with_raw_newline")]
    #[test_case(b"{{ [] }}" => Value::List(vec![]) ; "evaluates_empty_list")]
    #[test_case(b"{{ [1, \"x\", true] }}" => Value::List(vec![Value::Int(1), Value::Str("x".to_string()), Value::Bool(true)]) ; "evaluates_heterogeneous_list")]
    #[test_case(b"{{ [[1, 2], []] }}" => Value::List(vec![Value::List(vec![Value::Int(1), Value::Int(2)]), Value::List(vec![])]) ; "evaluates_nested_list")]
    #[test_case(b"{{ str }}" => Value::Str("foobar".to_string()) ; "evaluates_string_variable")]
    #[test_case(b"{{ num }}" => Value::Int(42) ; "evaluates_integer_variable")]
    #[test_case(b"{{ yes }}" => Value::Bool(true) ; "evaluates_boolean_variable")]
    #[test_case(b"{{ list }}" => Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)]) ; "evaluates_list_variable")]
    #[test_case(b"{{ map.key }}" => Value::Str("value".to_string()) ; "evaluates_map_field_access")]
    #[test_case(b"{{ map.nested.nested }}" => Value::Str("value".to_string()) ; "evaluates_nested_field_access")]
    #[test_case(b"{{ list.0 }}" => Value::Int(1) ; "evaluates_first_element_index")]
    #[test_case(b"{{ list.2 }}" => Value::Int(3) ; "evaluates_last_element_index")]
    #[test_case(b"{{ same([10, 20]).1 }}" => Value::Int(20) ; "evaluates_index_of_function_result")]
    #[test_case(b"{{ same(\"z\") }}" => Value::Str("z".to_string()) ; "evaluates_identity_function_call")]
    #[test_case(b"{{ foo(\"a\", \"b\") }}" => Value::Str("bar".to_string()) ; "evaluates_constant_function_call")]
    #[test_case(b"{{ foo() }}" => Value::Str("bar".to_string()) ; "evaluates_zero_argument_function_call")]
    #[test_case(b"{{ same(same(\"deep\")) }}" => Value::Str("deep".to_string()) ; "evaluates_nested_function_call")]
    fn evaluates_expressions(src: &[u8]) -> Value {
        let vars = var_scope();
        let scope = Scope::new(&vars);
        eval(&interp_expr(src), src, &scope, &TestRegistry).unwrap()
    }

    #[test_case(b"{{ missing }}" => RenderError::UndefinedVariable { name:"missing".to_string(), span: (3, 7).into() } ; "errors_on_undefined_variable")]
    #[test_case(b"{{ map.nope }}" => RenderError::MapKeyNotFound { key: "nope".into(), span: (7, 4).into() } ; "errors_on_missing_map_key")]
    #[test_case(b"{{ num.field }}" => RenderError::TypeMismatch { expected: ValueType::Map, got: ValueType::Int, span: (3, 3).into() } ; "rejects_field_access_on_int")]
    #[test_case(b"{{ yes.field }}" => RenderError::TypeMismatch { expected: ValueType::Map, got: ValueType::Bool, span: (3, 3).into() } ; "rejects_field_access_on_bool")]
    #[test_case(b"{{ list.field }}" => RenderError::TypeMismatch { expected: ValueType::Map, got: ValueType::List, span: (3, 4).into() } ; "rejects_field_access_on_list")]
    #[test_case(b"{{ [str].0.field }}" => RenderError::TypeMismatch { expected: ValueType::Map, got: ValueType::Str, span: (3, 7).into() } ; "rejects_field_access_on_str_list_element")]
    #[test_case(b"{{ \"s\".0 }}" => RenderError::TypeMismatch { expected: ValueType::List, got: ValueType::Str, span: (3, 3).into() } ; "rejects_index_access_on_str")]
    #[test_case(b"{{ map.0 }}" => RenderError::TypeMismatch { expected: ValueType::List, got: ValueType::Map, span: (3, 3).into() } ; "rejects_index_access_on_map")]
    #[test_case(b"{{ map.key.0 }}" => RenderError::TypeMismatch { expected: ValueType::List, got: ValueType::Str, span: (3, 7).into() } ; "rejects_index_access_on_field_value")]
    #[test_case(b"{{ list.3 }}" => RenderError::ListIndexOutOfBounds { idx: 3, len: 3, span: (8, 1).into() } ; "rejects_out_of_bounds_index")]
    #[test_case(b"{{ list.-1 }}" => RenderError::NegativeListIndex { idx: -1, span: (8, 2).into() } ; "rejects_negative_index")]
    #[test_case(b"{{ nope() }}" => RenderError::FunctionUndefined { name: "nope".into(), span: (3, 4).into() } ; "errors_on_undefined_function")]
    #[test_case(b"{{ one_arg() }}" => RenderError::FunctionArgCount { expected: 1, got: 0, span: (10, 2).into() } ; "rejects_missing_required_argument")]
    #[test_case(b"{{ one_arg(1, 2) }}" => RenderError::FunctionArgCount { expected: 1, got: 2, span: (12, 3).into() } ; "rejects_extra_argument")]
    #[test_case(b"{{ two_arg(1) }}" => RenderError::FunctionArgCount { expected: 2, got: 1, span: (12, 1).into() } ; "rejects_too_few_arguments")]
    #[test_case(b"{{ mismatch(12) }}" => RenderError::TypeMismatch { expected: ValueType::Str, got: ValueType::Int, span: (12, 2).into() } ; "rejects_wrong_argument_type")]
    #[test_case(b"{{ custom(1, 2) }}" => RenderError::Function { msg: "arguments must not match".into(), spans: vec![(10, 1).into(), (13, 1).into()] } ; "reports_custom_error_over_flagged_arguments")]
    #[test_case(b"{{ custom_empty() }}" => RenderError::Function { msg: "custom error with no flagged arguments".into(), spans: vec![(3, 14).into()] } ; "reports_custom_error_over_call_span")]
    fn errors_on_invalid_expressions(src: &[u8]) -> RenderError {
        let vars = var_scope();
        let scope = Scope::new(&vars);
        match eval(&interp_expr(src), src, &scope, &TestRegistry).unwrap_err() {
            Error::Render(err) => err,
            e => panic!("expected Render error, got {e:?}"),
        }
    }

    #[test_case(b"{% if yes %}Y{% end %}" => "Y"; "renders_true_branch")]
    #[test_case(b"{% if no %}Y{% else %}N{% end %}" => "N"; "renders_else_branch_when_false")]
    #[test_case(b"{% if no %}A{% elif yes %}B{% else %}C{% end %}" => "B"; "renders_taken_elif_branch")]
    #[test_case(b"{% if yes %}{% if yes %}ok{% end %}{% end %}" => "ok"; "renders_nested_if_branches")]
    #[test_case(b"{% for x in list %}{{x}},{% end %}" => "1,2,3,"; "iterates_list_variable")]
    #[test_case(b"{% for x in [10, 20] %}{{x}};{% end %}" => "10;20;"; "iterates_list_literal")]
    #[test_case(b"{% for x in same([\"a\", \"b\"]) %}{{x}}{% end %}" => "ab"; "iterates_list_from_function_call")]
    #[test_case(b"{% for x in list %}{{ str }}{% end %}" => "foobarfoobarfoobar"; "loop_body_reads_outer_variable")]
    #[test_case(b"{% for str in list %}{{str}}{% end %}" => "123"; "loop_variable_shadows_outer_binding")]
    #[test_case(b"{% for str in list %}{{str}}{% end %}{{str}}" => "123foobar"; "outer_binding_restored_after_loop")]
    #[test_case(b"{% for x in empty_list %}{{x}}{% end %}after" => "after"; "empty_iterable_renders_nothing")]
    #[test_case(b"{% if false %}{% for x in [1] %}{{ missing }}{% end %}{% end %}" => ""; "skips_for_body_in_untaken_branch")]
    fn renders_if_and_for_bodies(src: &[u8]) -> String {
        render(src, &var_scope()).unwrap()
    }

    #[test_case(b"{% if str %}x{% end %}" => RenderError::TypeMismatch { expected: ValueType::Bool, got: ValueType::Str, span: (6, 3).into() } ; "rejects_string_if_condition")]
    #[test_case(b"{% if no %}{% elif num %}x{% end %}" => RenderError::TypeMismatch { expected: ValueType::Bool, got: ValueType::Int, span: (19, 3).into() } ; "rejects_integer_elif_condition")]
    #[test_case(b"{% for x in str %}{{x}}{% end %}" => RenderError::TypeMismatch { expected: ValueType::List, got: ValueType::Str, span: (12, 3).into() } ; "rejects_string_for_iterable")]
    fn errors_on_invalid_control_flow_types(src: &[u8]) -> RenderError {
        match render(src, &var_scope()).unwrap_err() {
            Error::Render(err) => err,
            _ => panic!("expected Render error"),
        }
    }
}
