use std::ops::Range;

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

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Var(String),
    Str(String),
    Int(i64),
    Bool(bool),
    List(Vec<Expr>),
    FnCall { name: String, args: Vec<Expr> },
    Dot { left: Box<Expr>, field: String },
}
