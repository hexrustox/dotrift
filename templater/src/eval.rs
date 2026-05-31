use std::collections::HashMap;
use std::io;
use std::ops::Range;

use crate::ast::{Expr, ExprKind, Node};
use miette::{Context, Report, Result, miette};

use crate::error::{Error, ErrorKind, FuncError};
use crate::function::FunctionRegistry;
use crate::value::Value;

pub struct EvalContext<'a> {
    variables: HashMap<String, Value>,
    scopes: Vec<HashMap<String, Value>>,
    functions: &'a dyn FunctionRegistry,
}

impl<'a> EvalContext<'a> {
    pub fn new(variables: HashMap<String, Value>, functions: &'a dyn FunctionRegistry) -> Self {
        Self {
            variables,
            scopes: Vec::new(),
            functions,
        }
    }

    fn push_scope(&mut self, values: Vec<(String, Value)>) {
        self.scopes.push(HashMap::new());
        if let Some(scope) = self.scopes.last_mut() {
            for (name, value) in values.into_iter() {
                scope.insert(name, value);
            }
        }
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn get(&self, name: &str) -> Option<&Value> {
        for scope in self.scopes.iter().rev() {
            if let Some(val) = scope.get(name) {
                return Some(val);
            }
        }
        self.variables.get(name)
    }

    pub fn destruct(self) -> HashMap<String, Value> {
        self.variables
    }
}

pub fn eval_nodes<W: io::Write>(
    nodes: &[Node],
    source: &[u8],
    writer: &mut W,
    ctx: &mut EvalContext<'_>,
) -> Result<()> {
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
) -> Result<()> {
    match node {
        Node::Text(range) => {
            writer
                .write_all(&source[range.clone()])
                .map_err(|e| miette!("{e}"))
                .wrap_err("failed to write rendered template")?;
            Ok(())
        }
        Node::Interpolate(expr) => {
            let value = eval_expr(expr, ctx)?;
            value
                .write_to(writer)
                .map_err(|e| miette!("{e}"))
                .wrap_err("failed to write rendered template")?;
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
                        return Err(Report::new(Error::from_range(
                            ErrorKind::Function(FuncError::TypeMismatch {
                                arg: None,
                                expected: "Bool",
                                got: other.type_name(),
                            }),
                            cond.range.clone(),
                        )));
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
                        ctx.push_scope(vec![(var.clone(), item)]);
                        eval_nodes(body, source, writer, ctx)?;
                        ctx.pop_scope();
                    }
                    Ok(())
                }
                other => Err(Report::new(Error::from_range(
                    ErrorKind::Function(FuncError::TypeMismatch {
                        arg: None,
                        expected: "List",
                        got: other.type_name(),
                    }),
                    collection.range.clone(),
                ))),
            }
        }
    }
}

fn eval_expr(expr: &Expr, ctx: &EvalContext<'_>) -> Result<Value, Error> {
    match &expr.kind {
        ExprKind::Str(s) => Ok(Value::Str(s.clone())),
        ExprKind::Int(n) => Ok(Value::Int(*n)),
        ExprKind::Bool(b) => Ok(Value::Bool(*b)),
        ExprKind::Var(name) => ctx.get(name).cloned().ok_or(Error::from_range(
            ErrorKind::UndefinedVariable(name.clone()),
            expr.range.clone(),
        )),
        ExprKind::List(items) => {
            let vals: Result<Vec<_>, _> = items.iter().map(|e| eval_expr(e, ctx)).collect();
            Ok(Value::List(vals?))
        }
        ExprKind::FnCall { name, args } => {
            let arg_vals: Result<Vec<_>, _> = args.iter().map(|e| eval_expr(e, ctx)).collect();
            ctx.functions.call(name, &arg_vals?).map_err(|fe| match fe {
                FuncError::TypeMismatch { arg: Some(i), .. } => {
                    Error::from_range(ErrorKind::Function(fe), args[i].range.clone())
                }
                _ => Error::from_range(ErrorKind::Function(fe), expr.range.clone()),
            })
        }
        ExprKind::Dot { left, field } => {
            let val = eval_expr(left, ctx)?;
            let start = left.range.end;
            dot_access(val, field, start..start + field.len() + 1)
        }
    }
}

fn dot_access(val: Value, field: &str, range: Range<usize>) -> Result<Value, Error> {
    match val {
        Value::Map(mut map) => map.remove(field).ok_or(Error::from_range(
            ErrorKind::MapKeyNotFound(field.to_string()),
            range,
        )),
        Value::List(items) => {
            let index: usize = field.parse().map_err(|_| {
                Error::from_range(
                    ErrorKind::InvalidFieldAccess {
                        ty: "List",
                        field: field.to_string(),
                    },
                    range.clone(),
                )
            })?;
            items.get(index).cloned().ok_or(Error::from_range(
                ErrorKind::IndexOutOfBounds {
                    index,
                    len: items.len(),
                },
                range,
            ))
        }
        other => Err(Error::from_range(
            ErrorKind::InvalidFieldAccess {
                ty: other.type_name(),
                field: field.to_string(),
            },
            range,
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use test_case::test_case;

    use crate::{
        EvalContext, FuncError, Template, Value,
        error::{Error, ErrorKind},
        eval_nodes,
        function::FunctionRegistry,
        parser::parse,
        scanner::scan,
    };

    struct Functions;

    impl FunctionRegistry for Functions {
        fn call(&self, name: &str, args: &[Value]) -> Result<Value, FuncError> {
            match name {
                "add" => {
                    let mut total = 0i64;
                    for (i, v) in args.iter().enumerate() {
                        match v {
                            Value::Int(n) => total += n,
                            other => {
                                return Err(FuncError::TypeMismatch {
                                    arg: Some(i),
                                    expected: "Int",
                                    got: other.type_name(),
                                });
                            }
                        }
                    }
                    Ok(Value::Int(total))
                }
                "wrong_count" => Err(FuncError::WrongArgCount {
                    name: name.to_string(),
                    expected: "0".to_string(),
                    got: args.len(),
                }),
                other => Err(FuncError::Undefined(other.to_string())),
            }
        }
    }

    fn vars() -> HashMap<String, Value> {
        HashMap::from([
            ("name".to_string(), Value::Str("world".to_string())),
            ("count".to_string(), Value::Int(42)),
            ("neg".to_string(), Value::Int(-5)),
            ("flag".to_string(), Value::Bool(true)),
            ("off".to_string(), Value::Bool(false)),
            (
                "items".to_string(),
                Value::List(vec![
                    Value::Str("a".to_string()),
                    Value::Str("b".to_string()),
                ]),
            ),
            ("empty".to_string(), Value::List(vec![])),
            (
                "obj".to_string(),
                Value::Map(BTreeMap::from([(
                    "key".to_string(),
                    Value::Str("val".to_string()),
                )])),
            ),
        ])
    }

    #[test_case("hello" => "hello"; "plain_text")]
    #[test_case("" => ""; "empty")]
    #[test_case("{{ name }}" => "world"; "interpolate_var")]
    #[test_case("{{ count }}" => "42"; "interpolate_int")]
    #[test_case("{{ flag }}" => "true"; "interpolate_true")]
    #[test_case("{{ false }}" => "false"; "interpolate_false_literal")]
    #[test_case("{{ off }}" => "false"; "interpolate_false_var")]
    #[test_case("{{ -1 }}" => "-1"; "interpolate_negative_int")]
    #[test_case("{{ neg }}" => "-5"; "interpolate_neg_var")]
    #[test_case("{{ items }}" => "[a, b]"; "interpolate_list")]
    #[test_case("hello {{ name }}" => "hello world"; "mixed_text_and_interpolation")]
    #[test_case("{% if flag %}YES{% end %}" => "YES"; "if_true")]
    #[test_case("{% if off %}YES{% end %}" => ""; "if_false")]
    #[test_case("{% if off %}A{% else %}B{% end %}" => "B"; "if_else")]
    #[test_case("{% if off %}A{% elif flag %}B{% end %}" => "B"; "if_elif")]
    #[test_case("{% if off %}0{% elif off %}1{% elif flag %}2{% elif flag %}3{% end %}" => "2"; "if_multi_elif")]
    #[test_case("{% for x in items %}{{ x }}{% end %}" => "ab"; "for_list")]
    #[test_case("{% for x in empty %}{{ x }}{% end %}" => ""; "for_empty")]
    #[test_case("{% for x in [1, 2, 3] %}{{ x }}{% end %}" => "123"; "for_list_literal")]
    #[test_case("{% for name in items %}{{ name }}{% end %}" => "ab"; "for_shadow_outer")]
    #[test_case("{% for x in items %}{% for y in items %}{{ x }}{{ y }}{% end %}{% end %}" => "aaabbabb"; "nested_for")]
    #[test_case("{% if flag %}{% for x in items %}{{ x }}{% end %}{% end %}" => "ab"; "nested_if_for")]
    #[test_case("{{ items.0 }}" => "a"; "dot_list_index_0")]
    #[test_case("{{ items.1 }}" => "b"; "dot_list_index_1")]
    #[test_case("{{ obj.key }}" => "val"; "dot_map_key")]
    #[test_case("{{ add(1, 2) }}" => "3"; "fn_call_two_args")]
    #[test_case("{{ add(count, 8) }}" => "50"; "fn_call_with_var")]
    #[test_case("{{ add(add(1, 2), 3) }}" => "6"; "fn_call_nested")]
    fn test_render(template: &str) -> String {
        let tmpl = Template::from_bytes(template.to_owned().into_bytes()).unwrap();
        let mut buf = Vec::new();
        tmpl.render(&mut buf, vars(), &Functions).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test_case("{{ nothing }}" => (ErrorKind::UndefinedVariable("nothing".to_string()), 3, 7); "undefined_variable")]
    #[test_case("{{ items.99 }}" => (ErrorKind::IndexOutOfBounds { index: 99, len: 2 }, 8, 3); "index_out_of_bounds")]
    #[test_case("{{ name.0 }}" => (ErrorKind::InvalidFieldAccess { ty: "String", field: "0".to_string() }, 7, 2); "string_index_access")]
    #[test_case("{{ obj.missing }}" => (ErrorKind::MapKeyNotFound("missing".to_string()), 6, 8); "map_key_not_found")]
    #[test_case("{{ obj.key.missing }}" => (ErrorKind::InvalidFieldAccess { ty: "String", field: "missing".to_string() }, 10, 8); "string_key_access")]
    #[test_case("{{ items.x }}" => (ErrorKind::InvalidFieldAccess { ty: "List", field: "x".to_string() }, 8, 2); "invalid_field_access_list")]
    #[test_case("{{ count.field }}" => (ErrorKind::InvalidFieldAccess { ty: "Int", field: "field".to_string() }, 8, 6); "invalid_field_access_int")]
    #[test_case("{{ nofunc() }}" => (ErrorKind::Function(FuncError::Undefined("nofunc".to_string())), 3, 8); "function_undefined")]
    #[test_case("{{ wrong_count(1) }}" => (ErrorKind::Function(FuncError::WrongArgCount { name: "wrong_count".to_string(), expected: "0".to_string(), got: 1 }), 3, 14); "function_wrong_arg_count")]
    #[test_case("{{ add(true) }}" => (ErrorKind::Function(FuncError::TypeMismatch { arg: Some(0), expected: "Int", got: "Bool" }), 7, 4); "function_arg_type_mismatch_1")]
    #[test_case("{{ add(1, 2, true) }}" => (ErrorKind::Function(FuncError::TypeMismatch { arg: Some(2), expected: "Int", got: "Bool" }), 13, 4); "function_arg_type_mismatch_2")]
    #[test_case("{% if \"\" %}{% end %}" => (ErrorKind::Function(FuncError::TypeMismatch { arg: None, expected: "Bool", got: "String" }), 6, 2); "if_type_mismatch")]
    #[test_case("{% for x in \"\" %}{% end %}" => (ErrorKind::Function(FuncError::TypeMismatch { arg: None, expected: "List", got: "String" }), 12, 2); "for_type_mismatch")]
    fn test_error(template: &str) -> (ErrorKind, usize, usize) {
        let source = template.to_owned().into_bytes();
        let mut buf = Vec::new();
        let e = eval_nodes(
            &parse(&scan(&source).unwrap(), &source).unwrap(),
            &source,
            &mut buf,
            &mut EvalContext::new(vars(), &Functions),
        )
        .unwrap_err();
        e.downcast::<Error>().unwrap().destruct()
    }
}
