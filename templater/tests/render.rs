use std::collections::HashMap;

use templater::function::FunctionRegistry;
use templater::{EvalError, Template, Value};
use test_case::test_case;

struct Functions;

impl FunctionRegistry for Functions {
    fn call(&self, name: &str, args: &[Value]) -> Result<Value, EvalError> {
        match name {
            "add" => {
                let n = args
                    .iter()
                    .map(|v| match v {
                        Value::Int(n) => n,
                        _ => unimplemented!(),
                    })
                    .sum();
                Ok(Value::Int(n))
            }
            _ => unimplemented!(),
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
            Value::Map(HashMap::from([(
                "key".to_string(),
                Value::Str("val".to_string()),
            )])),
        ),
    ])
}

fn render(template: &str) -> String {
    let tmpl = Template::from_bytes(template.to_owned().into_bytes()).unwrap();
    let mut buf = Vec::new();
    tmpl.render(&mut buf, vars(), &Functions).unwrap();
    String::from_utf8(buf).unwrap()
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
    render(template)
}
