use std::collections::HashMap;
use std::io;

use crate::ast::{Expr, Node};
use crate::error::EvalError;
use crate::function::FunctionRegistry;
use crate::value::Value;

pub struct EvalContext<'a> {
    scopes: Vec<HashMap<String, Value>>,
    functions: &'a dyn FunctionRegistry,
}

impl<'a> EvalContext<'a> {
    pub fn new(variables: HashMap<String, Value>, functions: &'a dyn FunctionRegistry) -> Self {
        Self {
            scopes: vec![variables],
            functions,
        }
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub fn set(&mut self, name: String, value: Value) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, value);
        }
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        for scope in self.scopes.iter().rev() {
            if let Some(val) = scope.get(name) {
                return Some(val);
            }
        }
        None
    }
}

pub fn eval_nodes<W: io::Write>(
    nodes: &[Node],
    source: &[u8],
    writer: &mut W,
    ctx: &mut EvalContext<'_>,
) -> Result<(), EvalError> {
    for node in nodes {
        eval_node(node, source, writer, ctx)?;
    }
    Ok(())
}

fn eval_node<W: io::Write>(
    node: &Node,
    source: &[u8],
    writer: &mut W,
    ctx: &mut EvalContext<'_>,
) -> Result<(), EvalError> {
    match node {
        Node::Text(range) => {
            writer.write_all(&source[range.clone()])?;
            Ok(())
        }
        Node::Interpolate(expr) => {
            let value = eval_expr(expr, ctx)?;
            value.write_to(writer)?;
            Ok(())
        }
        Node::If {
            branches,
            else_branch,
        } => {
            for (cond, body) in branches {
                let val = eval_expr(cond, ctx)?;
                match val {
                    Value::Bool(true) => {
                        eval_nodes(body, source, writer, ctx)?;
                        return Ok(());
                    }
                    Value::Bool(false) => continue,
                    other => {
                        return Err(EvalError::TypeMismatch {
                            expected: "Bool",
                            got: other.type_name(),
                        });
                    }
                }
            }
            if let Some(body) = else_branch {
                eval_nodes(body, source, writer, ctx)?;
            }
            Ok(())
        }
        Node::For {
            var,
            collection,
            body,
        } => {
            let val = eval_expr(collection, ctx)?;
            match val {
                Value::List(items) => {
                    for item in items {
                        ctx.push_scope();
                        ctx.set(var.clone(), item);
                        eval_nodes(body, source, writer, ctx)?;
                        ctx.pop_scope();
                    }
                    Ok(())
                }
                other => Err(EvalError::TypeMismatch {
                    expected: "List",
                    got: other.type_name(),
                }),
            }
        }
    }
}

fn eval_expr(expr: &Expr, ctx: &EvalContext<'_>) -> Result<Value, EvalError> {
    match expr {
        Expr::Str(s) => Ok(Value::Str(s.clone())),
        Expr::Int(n) => Ok(Value::Int(*n)),
        Expr::Bool(b) => Ok(Value::Bool(*b)),
        Expr::Var(name) => ctx
            .get(name)
            .cloned()
            .ok_or_else(|| EvalError::UndefinedVariable(name.clone())),
        Expr::List(items) => {
            let vals: Result<Vec<_>, _> = items.iter().map(|e| eval_expr(e, ctx)).collect();
            Ok(Value::List(vals?))
        }
        Expr::FnCall { name, args } => {
            let arg_vals: Result<Vec<_>, _> = args.iter().map(|e| eval_expr(e, ctx)).collect();
            ctx.functions.call(name, &arg_vals?)
        }
        Expr::Dot { left, field } => {
            let val = eval_expr(left, ctx)?;
            dot_access(val, field)
        }
    }
}

fn dot_access(val: Value, field: &str) -> Result<Value, EvalError> {
    match val {
        Value::Map(mut map) => map
            .remove(field)
            .ok_or_else(|| EvalError::MapKeyNotFound(field.to_string())),
        Value::List(items) => {
            let index: usize = field.parse().map_err(|_| EvalError::InvalidFieldAccess {
                ty: "List",
                field: field.to_string(),
            })?;
            items
                .get(index)
                .cloned()
                .ok_or(EvalError::IndexOutOfBounds {
                    index,
                    len: items.len(),
                })
        }
        Value::Str(_) => Err(EvalError::StringIndexAccess),
        other => Err(EvalError::InvalidFieldAccess {
            ty: other.type_name(),
            field: field.to_string(),
        }),
    }
}
