use std::collections::HashMap;

use templater::{FuncError, Template, Value, function::FunctionRegistry};
use test_case::test_case;

struct Functions;

impl FunctionRegistry for Functions {
    fn call(&self, name: &str, _args: &[Value]) -> Result<Value, FuncError> {
        match name {
            "fn" => Err(FuncError::TypeMismatch {
                arg: Some(0),
                expected: "Type1",
                got: "Type2",
            }),
            other => Err(FuncError::Undefined(other.to_string())),
        }
    }
}

#[test_case("{{ unclosed"; "unclosed_delimiter")]
#[test_case("stray }}"; "stray_delimiter")]
#[test_case("{{ @ }}"; "unexpected_token")]
#[test_case("{%%}"; "empty_statement")]
#[test_case("{% foobar %}"; "unexpected_keyword")]
#[test_case("{% if true %}"; "unclosed_if_block")]
#[test_case("{{ \"hello }}";"unclosed_string")]
#[test_case("{% for %}"; "for_missing_var")]
#[test_case("{% for 123 in items %}"; "for_non_ident_var")]
#[test_case("{{ key.val }}"; "string_index_access")]
#[test_case("{% if 1 %}{% end %}"; "if_type_mismatch")]
#[test_case("{{ fn(1) }}";"function_type_mismatch")]
fn test_parse_error(template: &str) -> miette::Result<()> {
    let mut buf = Vec::new();
    Template::from_bytes(template.as_bytes().to_vec())?.render(
        &mut buf,
        HashMap::from_iter([("key".to_string(), Value::Str("val".to_string()))]),
        &Functions,
    )?;
    Ok(())
}
