use std::ops::Range;

/// A parsed template node. Ranges are byte offsets into the source.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Node {
    /// Plain text, emitted verbatim.
    Text(Range<usize>),
    /// `{{ expr }}` — the expression is evaluated and its value emitted.
    Interpolate(Expr),
    /// `{% if expr %} ... {% elif expr %} ... {% else %} ... {% end %}`.
    /// Branches are stored in source order; the first `Bool(true)` condition
    /// renders its body. `else_body` runs when no branch matched.
    If {
        branches: Vec<Branch>,
        else_body: Option<Vec<Node>>,
    },
    /// `{% for var in iter %} ... {% end %}`. `var` is the byte span of the
    /// loop-variable identifier; `iter` is evaluated once and must be a List.
    For {
        var: Range<usize>,
        iter: Expr,
        body: Vec<Node>,
    },
}

/// One branch of an `{% if %}` block: a condition expression and the body
/// that runs when the condition evaluates to `Bool(true)`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Branch {
    pub(crate) cond: Expr,
    pub(crate) body: Vec<Node>,
}

/// A parsed expression. Ranges are byte offsets into the source; string
/// literals keep their interior bytes as a range and are escape-walked on
/// render (zero allocation, zero owned `String` in the AST).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Expr {
    /// `<identifier>` — `name.0..name.1` is the identifier's byte span,
    /// resolved against the active scope at render time.
    Var(Range<usize>),
    /// `"..."` — `interior` is between the quotes; `span` covers the full
    /// literal including quotes (used for error attribution).
    StrLit {
        interior: Range<usize>,
        span: Range<usize>,
    },
    /// `<integer>` — decoded at parse time into `i64`. `span` is the
    /// literal's byte span for error attribution.
    IntLit { value: i64, span: Range<usize> },
    /// `true` | `false` — decoded at parse time into `bool`. `span` is the
    /// literal's byte span for error attribution.
    BoolLit { value: bool, span: Range<usize> },
    /// `[ expr, expr, ... ]` — `elements` are evaluated via `eval`; `span`
    /// covers the full list literal including brackets.
    List {
        elements: Vec<Expr>,
        span: Range<usize>,
    },
    /// `left.field` — Map key lookup by identifier; `field` is the
    /// identifier's byte span.
    Dot {
        left: Box<Expr>,
        field: Range<usize>,
    },
    /// `left.0` — List index lookup by non-negative integer; negative indices
    /// are stored and rejected at render time. `idx_span` covers the integer
    /// literal for error reporting.
    Index {
        left: Box<Expr>,
        idx: i64,
        idx_span: Range<usize>,
    },
    /// `name(args...)` — function call resolved against the host registry.
    /// `name` is the identifier's byte span; `paren` covers `(` through `)`.
    FnCall {
        name: Range<usize>,
        args: Vec<Expr>,
        paren: Range<usize>,
    },
}

impl Expr {
    /// The full byte span of the expression in the original source.
    pub(crate) fn span(&self) -> Range<usize> {
        match self {
            Expr::Var(range) => range.clone(),
            Expr::StrLit { span, .. } => span.clone(),
            Expr::IntLit { span, .. } | Expr::BoolLit { span, .. } => span.clone(),
            Expr::List { span, .. } => span.clone(),
            Expr::Dot { left, field } => left.span().start..field.end,
            Expr::Index { left, idx_span, .. } => left.span().start..idx_span.end,
            Expr::FnCall { name, paren, .. } => name.start..paren.end,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{parser::parse, scanner::scan};
    use test_case::test_case;

    use super::*;

    #[test_case(b"var" => 2..5 ; "simple_variable")]
    #[test_case(br#""hi""# => 2..6 ; "string_literal")]
    #[test_case(b"42" => 2..4 ; "positive_integer_literal")]
    #[test_case(b" -7" => 3..5 ; "negative_integer_literal")]
    #[test_case(b"[]" => 2..4 ; "empty_list")]
    #[test_case(b"[ a , b ]" => 2..11 ; "list_with_elements")]
    #[test_case(b"a.b.c" => 2..7 ; "chained_dot_access")]
    #[test_case(b"a.0.1" => 2..7 ; "mixed_dot_and_index_access")]
    #[test_case(b"f()" => 2..5 ; "function_call_no_args")]
    #[test_case(b"f( g( ) , h())" => 2..16 ; "nested_function_calls")]
    fn span(src: &[u8]) -> Range<usize> {
        let src = [b"{{", src, b"}}"].concat();
        let Node::Interpolate(expr) = parse(scan(&src).unwrap(), &src).unwrap().pop().unwrap()
        else {
            panic!("expected Interpolate")
        };
        expr.span()
    }
}
