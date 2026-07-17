use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Var(String),
    Str(String),
    Int(i64),
    Bool(bool),
    List(Vec<Expr>),
    FnCall { name: String, args: Vec<Expr> },
    Dot { left: Box<Expr>, field: String },
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub range: Range<usize>,
}

impl PartialEq for Expr {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Text(Range<usize>),
    Interpolate(Expr),
    If {
        branches: Vec<(Expr, Vec<Node>)>,
        else_branch: Option<Vec<Node>>,
    },
    For {
        var: String,
        collection: Expr,
        body: Vec<Node>,
    },
}
