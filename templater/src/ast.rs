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
}
