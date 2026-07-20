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
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Expr {
    /// `<identifier>` — `name.0..name.1` is the identifier's byte span,
    /// resolved against the active scope at render time.
    Var(Range<usize>),
    /// `"..."` — the range covers the literal's interior (between the
    /// opening and closing quotes, exclusive).
    StrLit(Range<usize>),
    /// `<integer>` — decoded at parse time into `i64`.
    IntLit(i64),
    /// `true` | `false` — decoded at parse time into `bool`.
    BoolLit(bool),
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
}
