use std::ops::Range;

/// A parsed template node. Ranges are byte offsets into the source.
#[derive(Debug, Clone)]
pub(crate) enum Node {
    /// Plain text, emitted verbatim.
    Text(Range<usize>),
    /// `{{ expr }}` — the expression is evaluated and its value emitted.
    Interpolate(Expr),
}

/// A parsed expression. Ranges are byte offsets into the source; string
/// literals keep their interior bytes as a range and are escape-walked on
/// render (zero allocation, zero owned `String` in the AST).
#[derive(Debug, Clone)]
pub(crate) enum Expr {
    /// `<identifier>` — `name.0..name.1` is the identifier's byte span,
    /// resolved against the active scope at render time.
    Var(Range<usize>),
    /// `"..."` — the range covers the literal's interior (between the
    /// opening and closing quotes, exclusive).
    StrLit(Range<usize>),
    /// `<integer>` — decoded at parse time into `i64`. `range` is the
    /// literal's byte span for error attribution.
    IntLit(i64, Range<usize>),
    /// `true` | `false` — decoded at parse time into `bool`. `range` is the
    /// literal's byte span for error attribution.
    BoolLit(bool, Range<usize>),
    /// `[ expr, expr, ... ]` — each element is evaluated via `eval`.
    List(Vec<Expr>),
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

impl PartialEq for Expr {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Expr::Var(a), Expr::Var(b)) => a == b,
            (Expr::StrLit(a), Expr::StrLit(b)) => a == b,
            // Literal source spans are structural (used for error reporting),
            // not part of the value, so they are ignored for equality.
            (Expr::IntLit(a, _), Expr::IntLit(b, _)) => a == b,
            (Expr::BoolLit(a, _), Expr::BoolLit(b, _)) => a == b,
            (Expr::List(a), Expr::List(b)) => a == b,
            (
                Expr::Dot {
                    left: l1,
                    field: f1,
                },
                Expr::Dot {
                    left: l2,
                    field: f2,
                },
            ) => l1 == l2 && f1 == f2,
            (
                Expr::Index {
                    left: l1,
                    idx: i1,
                    idx_span: s1,
                },
                Expr::Index {
                    left: l2,
                    idx: i2,
                    idx_span: s2,
                },
            ) => l1 == l2 && i1 == i2 && s1 == s2,
            (
                Expr::FnCall {
                    name: n1,
                    args: a1,
                    paren: p1,
                },
                Expr::FnCall {
                    name: n2,
                    args: a2,
                    paren: p2,
                },
            ) => n1 == n2 && a1 == a2 && p1 == p2,
            _ => false,
        }
    }
}

impl Eq for Expr {}

impl Expr {
    /// The full byte span of the expression in the original source.
    pub(crate) fn span(&self) -> Range<usize> {
        match self {
            Expr::Var(range)
            | Expr::StrLit(range)
            | Expr::FnCall {
                name: range,
                args: _,
                paren: _,
            } => range.clone(),
            Expr::IntLit(_, range) | Expr::BoolLit(_, range) => range.clone(),
            Expr::List(elements) => {
                if elements.is_empty() {
                    0..0
                } else {
                    elements.first().unwrap().span().start..elements.last().unwrap().span().end
                }
            }
            Expr::Dot { left: _, field } => field.clone(),
            Expr::Index {
                left: _,
                idx: _,
                idx_span,
            } => idx_span.clone(),
        }
    }
}
