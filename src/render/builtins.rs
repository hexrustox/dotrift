//! The builtin template functions (ADR-0017): the fixed function set filling
//! the templater's host-provided registry, identical for `dotrift.toml`
//! rendering and deployed templates.

use std::{collections::BTreeMap, env};

use templater::{
    error::RegistryError,
    function::FunctionRegistry,
    value::{Value, ValueType},
};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Builtins;

impl FunctionRegistry for Builtins {
    fn call(&self, name: &str, args: &[Value]) -> Result<Value, RegistryError> {
        match name {
            "env" => env_var(args),
            "home" => home_dir(args),
            "os" => {
                arg_count(args, 0)?;
                Ok(Value::Str(env::consts::OS.to_string()))
            }
            "arch" => {
                arg_count(args, 0)?;
                Ok(Value::Str(env::consts::ARCH.to_string()))
            }
            "upper" => {
                arg_count(args, 1)?;
                Ok(Value::Str(str_arg(args, 0)?.to_uppercase()))
            }
            "lower" => {
                arg_count(args, 1)?;
                Ok(Value::Str(str_arg(args, 0)?.to_lowercase()))
            }
            "trim" => {
                arg_count(args, 1)?;
                Ok(Value::Str(str_arg(args, 0)?.trim().to_string()))
            }
            "replace" => {
                arg_count(args, 3)?;
                let s = str_arg(args, 0)?;
                let from = str_arg(args, 1)?;
                let to = str_arg(args, 2)?;
                Ok(Value::Str(s.replace(from, to)))
            }
            "split" => {
                arg_count(args, 2)?;
                let s = str_arg(args, 0)?;
                let sep = str_arg(args, 1)?;
                Ok(Value::List(
                    s.split(sep)
                        .map(|part| Value::Str(part.to_string()))
                        .collect(),
                ))
            }
            "join" => join(args),
            "starts_with" => {
                arg_count(args, 2)?;
                let s = str_arg(args, 0)?;
                let prefix = str_arg(args, 1)?;
                Ok(Value::Bool(s.starts_with(prefix)))
            }
            "ends_with" => {
                arg_count(args, 2)?;
                let s = str_arg(args, 0)?;
                let suffix = str_arg(args, 1)?;
                Ok(Value::Bool(s.ends_with(suffix)))
            }
            "eq" => {
                arg_count(args, 2)?;
                Ok(Value::Bool(args[0] == args[1]))
            }
            "ne" => {
                arg_count(args, 2)?;
                Ok(Value::Bool(args[0] != args[1]))
            }
            "gt" => {
                arg_count(args, 2)?;
                Ok(Value::Bool(int_arg(args, 0)? > int_arg(args, 1)?))
            }
            "gte" => {
                arg_count(args, 2)?;
                Ok(Value::Bool(int_arg(args, 0)? >= int_arg(args, 1)?))
            }
            "lt" => {
                arg_count(args, 2)?;
                Ok(Value::Bool(int_arg(args, 0)? < int_arg(args, 1)?))
            }
            "lte" => {
                arg_count(args, 2)?;
                Ok(Value::Bool(int_arg(args, 0)? <= int_arg(args, 1)?))
            }
            "add" => {
                min_arg_count(args, 2)?;
                Ok(Value::Int(int_args(args)?.into_iter().sum()))
            }
            "sub" => {
                arg_count(args, 2)?;
                Ok(Value::Int(int_arg(args, 0)? - int_arg(args, 1)?))
            }
            "mul" => {
                min_arg_count(args, 2)?;
                Ok(Value::Int(int_args(args)?.into_iter().product()))
            }
            "div" => {
                arg_count(args, 2)?;
                let b = int_arg(args, 1)?;
                if b == 0 {
                    return Err(custom("division by zero", &[]));
                }
                Ok(Value::Int(int_arg(args, 0)? / b))
            }
            "neg" => {
                arg_count(args, 1)?;
                Ok(Value::Int(-int_arg(args, 0)?))
            }
            "and" => {
                min_arg_count(args, 2)?;
                fold_bools(args, true, false)
            }
            "or" => {
                min_arg_count(args, 2)?;
                fold_bools(args, false, true)
            }
            "not" => {
                arg_count(args, 1)?;
                Ok(Value::Bool(!bool_arg(args, 0)?))
            }
            "coalesce" => {
                min_arg_count(args, 1)?;
                Ok(args
                    .iter()
                    .find(|value| truthy(value))
                    .unwrap_or_else(|| args.last().expect("checked non-empty"))
                    .clone())
            }
            "is_truthy" => {
                arg_count(args, 1)?;
                Ok(Value::Bool(truthy(&args[0])))
            }
            "to_str" => {
                arg_count(args, 1)?;
                Ok(Value::Str(to_str(&args[0])))
            }
            "to_int" => {
                arg_count(args, 1)?;
                Ok(Value::Int(to_int(&args[0])?))
            }
            "length" => {
                arg_count(args, 1)?;
                Ok(Value::Int(length(&args[0])?))
            }
            "contains" => {
                arg_count(args, 2)?;
                Ok(Value::Bool(contains(args)?))
            }
            "first" => {
                arg_count(args, 1)?;
                list_arg(args, 0)?
                    .first()
                    .cloned()
                    .ok_or_else(|| custom("cannot take `first` of an empty list", &[]))
            }
            "last" => {
                arg_count(args, 1)?;
                list_arg(args, 0)?
                    .last()
                    .cloned()
                    .ok_or_else(|| custom("cannot take `last` of an empty list", &[]))
            }
            "keys" => {
                arg_count(args, 1)?;
                Ok(Value::List(
                    map_arg(args, 0)?
                        .keys()
                        .map(|key| Value::Str(key.clone()))
                        .collect(),
                ))
            }
            "values" => {
                arg_count(args, 1)?;
                Ok(Value::List(map_arg(args, 0)?.values().cloned().collect()))
            }
            "enumerate" => {
                arg_count(args, 1)?;
                Ok(Value::List(enumerate(list_arg(args, 0)?)))
            }
            _ => Err(RegistryError::Undefined { name: name.into() }),
        }
    }
}

fn arg(args: &[Value], index: usize, expected: ValueType) -> Result<&Value, RegistryError> {
    let arg = &args[index];
    if arg.value_type() == expected {
        Ok(arg)
    } else {
        Err(RegistryError::TypeMismatch {
            expected,
            got: arg.value_type(),
            arg_index: index,
        })
    }
}

fn arg_count(args: &[Value], expected: usize) -> Result<(), RegistryError> {
    if args.len() == expected {
        Ok(())
    } else {
        Err(RegistryError::ArgCount {
            expected,
            got: args.len(),
        })
    }
}

fn min_arg_count(args: &[Value], expected: usize) -> Result<(), RegistryError> {
    if args.len() >= expected {
        Ok(())
    } else {
        Err(RegistryError::ArgCount {
            expected,
            got: args.len(),
        })
    }
}

fn str_arg(args: &[Value], index: usize) -> Result<&str, RegistryError> {
    match arg(args, index, ValueType::Str)? {
        Value::Str(s) => Ok(s),
        _ => unreachable!("arg already checked the value type"),
    }
}

fn int_arg(args: &[Value], index: usize) -> Result<i64, RegistryError> {
    match arg(args, index, ValueType::Int)? {
        Value::Int(n) => Ok(*n),
        _ => unreachable!("arg already checked the value type"),
    }
}

fn bool_arg(args: &[Value], index: usize) -> Result<bool, RegistryError> {
    match arg(args, index, ValueType::Bool)? {
        Value::Bool(b) => Ok(*b),
        _ => unreachable!("arg already checked the value type"),
    }
}

fn list_arg(args: &[Value], index: usize) -> Result<&[Value], RegistryError> {
    match arg(args, index, ValueType::List)? {
        Value::List(items) => Ok(items),
        _ => unreachable!("arg already checked the value type"),
    }
}

fn map_arg(args: &[Value], index: usize) -> Result<&BTreeMap<String, Value>, RegistryError> {
    match arg(args, index, ValueType::Map)? {
        Value::Map(map) => Ok(map),
        _ => unreachable!("arg already checked the value type"),
    }
}

fn int_args(args: &[Value]) -> Result<Vec<i64>, RegistryError> {
    (0..args.len()).map(|index| int_arg(args, index)).collect()
}

fn custom(msg: impl Into<String>, indexes: &[usize]) -> RegistryError {
    RegistryError::Custom {
        msg: msg.into(),
        indexes: indexes.to_vec(),
    }
}

fn receiver(msg: impl Into<String>) -> RegistryError {
    custom(msg, &[0])
}

fn fold_bools(args: &[Value], start: bool, stop: bool) -> Result<Value, RegistryError> {
    for index in 0..args.len() {
        if bool_arg(args, index)? == stop {
            return Ok(Value::Bool(stop));
        }
    }
    Ok(Value::Bool(start))
}

fn env_var(args: &[Value]) -> Result<Value, RegistryError> {
    arg_count(args, 2)?;
    let var = str_arg(args, 0)?;
    let fallback = str_arg(args, 1)?;
    Ok(Value::Str(
        env::var(var).unwrap_or_else(|_| fallback.to_string()),
    ))
}

fn home_dir(args: &[Value]) -> Result<Value, RegistryError> {
    arg_count(args, 0)?;
    let home = env::var("HOME")
        .ok()
        .filter(|home| !home.is_empty())
        .or_else(|| dirs::home_dir().map(|path| path.to_string_lossy().into_owned()))
        .unwrap_or_default();
    Ok(Value::Str(home))
}

fn join(args: &[Value]) -> Result<Value, RegistryError> {
    min_arg_count(args, 2)?;
    let sep = str_arg(args, 0)?;
    let parts = (1..args.len())
        .map(|index| str_arg(args, index))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Str(parts.join(sep)))
}

fn to_int(value: &Value) -> Result<i64, RegistryError> {
    match value {
        Value::Int(n) => Ok(*n),
        Value::Bool(b) => Ok(i64::from(*b)),
        Value::Str(s) => s
            .parse()
            .map_err(|_| custom(format!("cannot convert {s:?} to an integer"), &[0])),
        Value::List(_) => Err(receiver("cannot convert a list to an integer")),
        Value::Map(_) => Err(receiver("cannot convert a map to an integer")),
    }
}

fn to_str(value: &Value) -> String {
    match value {
        Value::Str(s) => s.clone(),
        Value::Int(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::List(items) => {
            let joined = items.iter().map(to_str).collect::<Vec<_>>().join(", ");
            format!("[{joined}]")
        }
        Value::Map(map) => {
            let joined = map
                .iter()
                .map(|(key, value)| format!("{key}: {}", to_str(value)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{{joined}}}")
        }
    }
}

fn length(value: &Value) -> Result<i64, RegistryError> {
    match value {
        Value::Str(s) => Ok(s.len() as i64),
        Value::List(items) => Ok(items.len() as i64),
        Value::Map(map) => Ok(map.len() as i64),
        Value::Int(_) | Value::Bool(_) => Err(receiver(
            "cannot take the length of a non-string, non-list, non-map value",
        )),
    }
}

fn contains(args: &[Value]) -> Result<bool, RegistryError> {
    match &args[0] {
        Value::Str(s) => Ok(s.contains(str_arg(args, 1)?)),
        Value::List(items) => Ok(items.contains(&args[1])),
        Value::Map(map) => Ok(map.contains_key(str_arg(args, 1)?)),
        Value::Int(_) | Value::Bool(_) => Err(receiver(
            "cannot search in a non-string, non-list, non-map value",
        )),
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Str(s) => !s.is_empty(),
        Value::Int(n) => *n != 0,
        Value::Bool(b) => *b,
        Value::List(items) => !items.is_empty(),
        Value::Map(map) => !map.is_empty(),
    }
}

fn enumerate(items: &[Value]) -> Vec<Value> {
    items
        .iter()
        .enumerate()
        .map(|(index, value)| {
            Value::Map(BTreeMap::from([
                ("index".to_string(), Value::Int(index as i64)),
                ("value".to_string(), value.clone()),
            ]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;

    fn call(name: &str, args: &[Value]) -> Result<Value, RegistryError> {
        Builtins.call(name, args)
    }

    fn str_value(value: &str) -> Value {
        Value::Str(value.to_string())
    }

    fn list(values: Vec<Value>) -> Value {
        Value::List(values)
    }

    fn map(entries: &[(&str, Value)]) -> Value {
        Value::Map(BTreeMap::from_iter(
            entries
                .iter()
                .map(|(key, value)| (key.to_string(), value.clone())),
        ))
    }

    #[test_case("eq", vec![str_value("a"), str_value("a")] => Ok(Value::Bool(true)); "eq matches same strings")]
    #[test_case("eq", vec![str_value("1"), Value::Int(1)] => Ok(Value::Bool(false)); "eq never matches across types")]
    #[test_case("ne", vec![str_value("a"), str_value("b")] => Ok(Value::Bool(true)); "ne distinguishes strings")]
    #[test_case("gt", vec![Value::Int(3), Value::Int(2)] => Ok(Value::Bool(true)); "gt compares ints")]
    #[test_case("gte", vec![Value::Int(2), Value::Int(2)] => Ok(Value::Bool(true)); "gte allows equal ints")]
    #[test_case("lt", vec![Value::Int(2), Value::Int(3)] => Ok(Value::Bool(true)); "lt compares ints")]
    #[test_case("lte", vec![Value::Int(2), Value::Int(2)] => Ok(Value::Bool(true)); "lte allows equal ints")]
    #[test_case("add", vec![Value::Int(1), Value::Int(2), Value::Int(3)] => Ok(Value::Int(6)); "add folds variadic ints")]
    #[test_case("sub", vec![Value::Int(5), Value::Int(2)] => Ok(Value::Int(3)); "sub subtracts ints")]
    #[test_case("mul", vec![Value::Int(2), Value::Int(3), Value::Int(4)] => Ok(Value::Int(24)); "mul folds variadic ints")]
    #[test_case("div", vec![Value::Int(7), Value::Int(2)] => Ok(Value::Int(3)); "div truncates toward zero")]
    #[test_case("neg", vec![Value::Int(5)] => Ok(Value::Int(-5)); "neg negates an int")]
    #[test_case("upper", vec![str_value("aBc")] => Ok(str_value("ABC")); "upper uppercases a string")]
    #[test_case("lower", vec![str_value("aBc")] => Ok(str_value("abc")); "lower lowercases a string")]
    #[test_case("trim", vec![str_value("  x  ")] => Ok(str_value("x")); "trim strips surrounding whitespace")]
    #[test_case("replace", vec![str_value("aXa"), str_value("X"), str_value("Y")] => Ok(str_value("aYa")); "replace swaps all occurrences")]
    #[test_case("split", vec![str_value("a,,b"), str_value(",")] => Ok(list(vec![str_value("a"), str_value(""), str_value("b")])); "split keeps empty parts")]
    #[test_case("join", vec![str_value(", "), str_value("a"), str_value("b")] => Ok(str_value("a, b")); "join interleaves the separator")]
    #[test_case("join", vec![str_value("-"), str_value("only")] => Ok(str_value("only")); "join of one part needs no separator")]
    #[test_case("starts_with", vec![str_value("abc"), str_value("ab")] => Ok(Value::Bool(true)); "starts_with matches a prefix")]
    #[test_case("ends_with", vec![str_value("abc"), str_value("ab")] => Ok(Value::Bool(false)); "ends_with rejects a non-suffix")]
    #[test_case("not", vec![Value::Bool(true)] => Ok(Value::Bool(false)); "not negates a bool")]
    #[test_case("and", vec![Value::Bool(true), Value::Bool(false)] => Ok(Value::Bool(false)); "and requires all true")]
    #[test_case("or", vec![Value::Bool(false), Value::Bool(true)] => Ok(Value::Bool(true)); "or passes on any true")]
    #[test_case("is_truthy", vec![str_value("")] => Ok(Value::Bool(false)); "is_truthy rejects an empty string")]
    #[test_case("is_truthy", vec![Value::Int(0)] => Ok(Value::Bool(false)); "is_truthy rejects zero")]
    #[test_case("is_truthy", vec![Value::Bool(true)] => Ok(Value::Bool(true)); "is_truthy passes a true bool")]
    #[test_case("is_truthy", vec![list(vec![])] => Ok(Value::Bool(false)); "is_truthy rejects an empty list")]
    #[test_case("is_truthy", vec![list(vec![Value::Int(0)])] => Ok(Value::Bool(true)); "is_truthy passes a nonempty list")]
    #[test_case("coalesce", vec![str_value(""), str_value("fb"), str_value("x")] => Ok(str_value("fb")); "coalesce returns the first truthy")]
    #[test_case("coalesce", vec![str_value(""), str_value("")] => Ok(str_value("")); "coalesce falls back to the last arg")]
    #[test_case("to_str", vec![Value::Int(-3)] => Ok(str_value("-3")); "to_str stringifies an int")]
    #[test_case("to_str", vec![Value::Bool(true)] => Ok(str_value("true")); "to_str stringifies a bool")]
    #[test_case("to_str", vec![list(vec![Value::Int(1), str_value("a")])] => Ok(str_value("[1, a]")); "to_str stringifies a list")]
    #[test_case("to_str", vec![map(&[("k", str_value("v"))])] => Ok(str_value("{k: v}")); "to_str stringifies a map")]
    #[test_case("to_int", vec![Value::Int(3)] => Ok(Value::Int(3)); "to_int keeps ints")]
    #[test_case("to_int", vec![Value::Bool(true)] => Ok(Value::Int(1)); "to_int maps true to one")]
    #[test_case("to_int", vec![str_value("42")] => Ok(Value::Int(42)); "to_int parses decimal strings")]
    #[test_case("length", vec![str_value("abc")] => Ok(Value::Int(3)); "length counts string bytes")]
    #[test_case("length", vec![list(vec![Value::Int(1), Value::Int(2)])] => Ok(Value::Int(2)); "length counts list items")]
    #[test_case("length", vec![map(&[("a", Value::Int(1)), ("b", Value::Int(2))])] => Ok(Value::Int(2)); "length counts map entries")]
    #[test_case("contains", vec![str_value("abc"), str_value("b")] => Ok(Value::Bool(true)); "contains searches strings")]
    #[test_case("contains", vec![list(vec![str_value("a")]), str_value("a")] => Ok(Value::Bool(true)); "contains searches lists")]
    #[test_case("contains", vec![list(vec![str_value("a")]), str_value("b")] => Ok(Value::Bool(false)); "contains reports missing list items")]
    #[test_case("first", vec![list(vec![str_value("a"), str_value("b")])] => Ok(str_value("a")); "first returns the head")]
    #[test_case("last", vec![list(vec![str_value("a"), str_value("b")])] => Ok(str_value("b")); "last returns the tail")]
    #[test_case("keys", vec![map(&[("b", Value::Int(2)), ("a", Value::Int(1))])] => Ok(list(vec![str_value("a"), str_value("b")])); "keys lists sorted keys")]
    #[test_case("values", vec![map(&[("b", Value::Int(2)), ("a", Value::Int(1))])] => Ok(list(vec![Value::Int(1), Value::Int(2)])); "values lists values in key order")]
    fn evaluates_functions(name: &str, args: Vec<Value>) -> Result<Value, RegistryError> {
        call(name, &args)
    }

    #[test_case("upper", vec![]; "upper rejects zero args")]
    #[test_case("upper", vec![str_value("x"), str_value("y")]; "upper rejects two args")]
    #[test_case("os", vec![str_value("x")]; "os rejects an arg")]
    #[test_case("env", vec![str_value("x")]; "env rejects one arg")]
    #[test_case("add", vec![Value::Int(1)]; "add rejects one arg")]
    #[test_case("and", vec![Value::Bool(true)]; "and rejects one arg")]
    #[test_case("coalesce", vec![]; "coalesce rejects zero args")]
    fn rejects_wrong_arg_counts(name: &str, args: Vec<Value>) {
        assert!(matches!(
            call(name, &args),
            Err(RegistryError::ArgCount { .. })
        ));
    }

    #[test_case("upper", vec![Value::Int(1)]; "upper rejects an int")]
    #[test_case("lower", vec![Value::Bool(true)]; "lower rejects a bool")]
    #[test_case("trim", vec![list(vec![])]; "trim rejects a list")]
    #[test_case("replace", vec![Value::Int(1), str_value("x"), str_value("y")]; "replace rejects an int receiver")]
    #[test_case("split", vec![str_value("x"), Value::Int(1)]; "split rejects an int separator")]
    #[test_case("starts_with", vec![Value::Int(1), str_value("x")]; "starts_with rejects an int receiver")]
    #[test_case("ends_with", vec![str_value("x"), Value::Int(1)]; "ends_with rejects an int suffix")]
    #[test_case("gt", vec![str_value("1"), Value::Int(2)]; "gt rejects a string")]
    #[test_case("add", vec![Value::Int(1), str_value("2")]; "add rejects a string operand")]
    #[test_case("div", vec![str_value("1"), Value::Int(2)]; "div rejects a string operand")]
    #[test_case("neg", vec![str_value("1")]; "neg rejects a string")]
    #[test_case("not", vec![Value::Int(1)]; "not rejects an int")]
    #[test_case("and", vec![Value::Bool(true), Value::Int(1)]; "and rejects an int")]
    #[test_case("or", vec![Value::Int(0), Value::Bool(true)]; "or rejects an int")]
    #[test_case("first", vec![str_value("x")]; "first rejects a string")]
    #[test_case("last", vec![Value::Int(3)]; "last rejects an int")]
    #[test_case("keys", vec![list(vec![])]; "keys rejects a list")]
    #[test_case("values", vec![Value::Int(1)]; "values rejects an int")]
    #[test_case("enumerate", vec![str_value("x")]; "enumerate rejects a string")]
    fn rejects_wrong_types(name: &str, args: Vec<Value>) {
        assert!(matches!(
            call(name, &args),
            Err(RegistryError::TypeMismatch { .. })
        ));
    }

    #[test]
    fn div_by_zero_is_a_custom_error() {
        let error = call("div", &[Value::Int(1), Value::Int(0)]).unwrap_err();
        assert_eq!(
            error,
            RegistryError::Custom {
                msg: "division by zero".into(),
                indexes: vec![]
            }
        );
    }

    #[test]
    fn to_int_reports_unparseable_strings() {
        let error = call("to_int", &[str_value("12x")]).unwrap_err();
        assert_eq!(
            error,
            RegistryError::Custom {
                msg: "cannot convert \"12x\" to an integer".into(),
                indexes: vec![0]
            }
        );
    }

    #[test]
    fn to_int_rejects_collections() {
        let error = call("to_int", &[list(vec![])]).unwrap_err();
        assert!(matches!(error, RegistryError::Custom { indexes, .. } if indexes == vec![0]));
    }

    #[test]
    fn first_and_last_report_empty_lists() {
        let first = call("first", &[list(vec![])]).unwrap_err();
        let last = call("last", &[list(vec![])]).unwrap_err();
        assert_eq!(
            first,
            RegistryError::Custom {
                msg: "cannot take `first` of an empty list".into(),
                indexes: vec![]
            }
        );
        assert_eq!(
            last,
            RegistryError::Custom {
                msg: "cannot take `last` of an empty list".into(),
                indexes: vec![]
            }
        );
    }

    #[test]
    fn length_rejects_scalars() {
        let error = call("length", &[Value::Bool(true)]).unwrap_err();
        assert!(matches!(error, RegistryError::Custom { indexes, .. } if indexes == vec![0]));
    }

    #[test]
    fn contains_rejects_scalars() {
        let error = call("contains", &[Value::Int(1), str_value("x")]).unwrap_err();
        assert!(matches!(error, RegistryError::Custom { indexes, .. } if indexes == vec![0]));
    }

    #[test]
    fn enumerate_wraps_items_in_index_value_maps() {
        let result = call("enumerate", &[list(vec![str_value("a"), Value::Int(1)])]).unwrap();
        let expected = list(vec![
            map(&[("index", Value::Int(0)), ("value", str_value("a"))]),
            map(&[("index", Value::Int(1)), ("value", Value::Int(1))]),
        ]);
        assert_eq!(result, expected);
    }

    #[test]
    fn env_reads_the_process_environment() {
        let value = env::var("HOME").expect("HOME must be set for tests");
        assert_eq!(
            call("env", &[str_value("HOME"), str_value("fb")]).unwrap(),
            str_value(&value)
        );
    }

    #[test]
    fn env_falls_back_when_unset() {
        assert_eq!(
            call(
                "env",
                &[str_value("DOTRIFT_TEST_BUILTIN_MISSING"), str_value("fb")]
            )
            .unwrap(),
            str_value("fb")
        );
    }

    #[test]
    fn host_facts_return_strings() {
        for name in ["home", "os", "arch"] {
            assert!(matches!(call(name, &[]).unwrap(), Value::Str(_)));
        }
    }

    #[test]
    fn undefined_names_are_undefined() {
        assert!(matches!(
            call("nope", &[]),
            Err(RegistryError::Undefined { .. })
        ));
    }
}
