//! The builtin template functions (ADR-0021): the fixed function set filling
//! the templater's host-provided registry, identical for `dotrift.toml`
//! rendering and deployed templates.

use std::{collections::BTreeMap, env, env::consts::ARCH, env::consts::OS};

use templater::error::RegistryError;
use templater::function::FunctionRegistry;
use templater::value::{Value, ValueType};

/// The dotrift builtin functions.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Builtins;

fn truthy(value: &Value) -> bool {
    match value {
        Value::Str(s) => !s.is_empty(),
        Value::Int(n) => *n != 0,
        Value::Bool(b) => *b,
        Value::List(items) => !items.is_empty(),
        Value::Map(map) => !map.is_empty(),
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

fn str_arg(args: &[Value], index: usize) -> Result<&str, RegistryError> {
    let Value::Str(s) = arg(args, index, ValueType::Str)? else {
        unreachable!("arg already checked the value type")
    };
    Ok(s)
}

fn int_arg(args: &[Value], index: usize) -> Result<i64, RegistryError> {
    let Value::Int(n) = arg(args, index, ValueType::Int)? else {
        unreachable!("arg already checked the value type")
    };
    Ok(*n)
}

fn bool_arg(args: &[Value], index: usize) -> Result<bool, RegistryError> {
    let Value::Bool(b) = arg(args, index, ValueType::Bool)? else {
        unreachable!("arg already checked the value type")
    };
    Ok(*b)
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

fn to_int(value: &Value) -> Result<i64, RegistryError> {
    match value {
        Value::Int(n) => Ok(*n),
        Value::Bool(b) => Ok(i64::from(*b)),
        Value::Str(s) => s
            .parse()
            .map_err(|_| custom(format!("cannot convert \"{s}\" to Int"), &[0])),
        Value::List(_) | Value::Map(_) => Err(receiver(
            "provide an Int, a Bool, or a String containing an integer",
        )),
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
            "provide a String, a List, or a Map to take the length of",
        )),
    }
}

fn contains(args: &[Value]) -> Result<bool, RegistryError> {
    match &args[0] {
        Value::Str(s) => Ok(s.contains(str_arg(args, 1)?)),
        Value::List(items) => Ok(items.contains(&args[1])),
        Value::Map(map) => Ok(map.contains_key(str_arg(args, 1)?)),
        Value::Int(_) | Value::Bool(_) => {
            Err(receiver("provide a String, a List, or a Map to search in"))
        }
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

impl Builtins {
    fn env(&self, args: &[Value]) -> Result<Value, RegistryError> {
        arg_count(args, 2)?;
        let var = str_arg(args, 0)?;
        let fallback = str_arg(args, 1)?;
        Ok(Value::Str(
            env::var(var).unwrap_or_else(|_| fallback.to_string()),
        ))
    }

    fn home(&self, args: &[Value]) -> Result<Value, RegistryError> {
        arg_count(args, 0)?;
        let home = env::var("HOME")
            .ok()
            .filter(|home| !home.is_empty())
            .or_else(|| dirs::home_dir().map(|path| path.to_string_lossy().into_owned()))
            .unwrap_or_default();
        Ok(Value::Str(home))
    }

    fn join(&self, args: &[Value]) -> Result<Value, RegistryError> {
        min_arg_count(args, 2)?;
        let sep = str_arg(args, 0)?;
        let parts = (1..args.len())
            .map(|index| str_arg(args, index))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Value::Str(parts.join(sep)))
    }

    fn add(&self, args: &[Value]) -> Result<Value, RegistryError> {
        min_arg_count(args, 2)?;
        Ok(Value::Int(int_args(args)?.into_iter().sum()))
    }

    fn mul(&self, args: &[Value]) -> Result<Value, RegistryError> {
        min_arg_count(args, 2)?;
        Ok(Value::Int(int_args(args)?.into_iter().product()))
    }

    fn sub(&self, args: &[Value]) -> Result<Value, RegistryError> {
        arg_count(args, 2)?;
        Ok(Value::Int(int_arg(args, 0)? - int_arg(args, 1)?))
    }

    fn div(&self, args: &[Value]) -> Result<Value, RegistryError> {
        arg_count(args, 2)?;
        let a = int_arg(args, 0)?;
        let b = int_arg(args, 1)?;
        if b == 0 {
            return Err(custom("division by zero", &[]));
        }
        Ok(Value::Int(a / b))
    }

    fn and(&self, args: &[Value]) -> Result<Value, RegistryError> {
        min_arg_count(args, 2)?;
        for index in 0..args.len() {
            if !bool_arg(args, index)? {
                return Ok(Value::Bool(false));
            }
        }
        Ok(Value::Bool(true))
    }

    fn or(&self, args: &[Value]) -> Result<Value, RegistryError> {
        min_arg_count(args, 2)?;
        for index in 0..args.len() {
            if bool_arg(args, index)? {
                return Ok(Value::Bool(true));
            }
        }
        Ok(Value::Bool(false))
    }

    fn coalesce(&self, args: &[Value]) -> Result<Value, RegistryError> {
        min_arg_count(args, 1)?;
        Ok(args
            .iter()
            .find(|value| truthy(value))
            .unwrap_or_else(|| args.last().expect("checked non-empty"))
            .clone())
    }
}

impl FunctionRegistry for Builtins {
    fn call(&self, name: &str, args: &[Value]) -> Result<Value, RegistryError> {
        match name {
            "env" => self.env(args),
            "home" => self.home(args),
            "os" => {
                arg_count(args, 0)?;
                Ok(Value::Str(OS.to_string()))
            }
            "arch" => {
                arg_count(args, 0)?;
                Ok(Value::Str(ARCH.to_string()))
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
            "join" => self.join(args),
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
            "add" => self.add(args),
            "sub" => self.sub(args),
            "mul" => self.mul(args),
            "div" => self.div(args),
            "neg" => {
                arg_count(args, 1)?;
                Ok(Value::Int(-int_arg(args, 0)?))
            }
            "and" => self.and(args),
            "or" => self.or(args),
            "not" => {
                arg_count(args, 1)?;
                Ok(Value::Bool(!bool_arg(args, 0)?))
            }
            "coalesce" => self.coalesce(args),
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
                let Value::List(items) = arg(args, 0, ValueType::List)? else {
                    unreachable!("arg already checked the value type")
                };
                items
                    .first()
                    .cloned()
                    .ok_or_else(|| custom("first of an empty list", &[]))
            }
            "last" => {
                arg_count(args, 1)?;
                let Value::List(items) = arg(args, 0, ValueType::List)? else {
                    unreachable!("arg already checked the value type")
                };
                items
                    .last()
                    .cloned()
                    .ok_or_else(|| custom("last of an empty list", &[]))
            }
            "keys" => {
                arg_count(args, 1)?;
                let Value::Map(map) = arg(args, 0, ValueType::Map)? else {
                    unreachable!("arg already checked the value type")
                };
                Ok(Value::List(
                    map.keys().map(|key| Value::Str(key.clone())).collect(),
                ))
            }
            "values" => {
                arg_count(args, 1)?;
                let Value::Map(map) = arg(args, 0, ValueType::Map)? else {
                    unreachable!("arg already checked the value type")
                };
                Ok(Value::List(map.values().cloned().collect()))
            }
            "enumerate" => {
                arg_count(args, 1)?;
                let Value::List(items) = arg(args, 0, ValueType::List)? else {
                    unreachable!("arg already checked the value type")
                };
                Ok(Value::List(enumerate(items)))
            }
            _ => Err(RegistryError::Undefined { name: name.into() }),
        }
    }
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;

    fn call(name: &str, args: &[Value]) -> Result<Value, RegistryError> {
        Builtins.call(name, args)
    }

    fn s(value: &str) -> Value {
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

    #[test_case("eq", vec![s("a"), s("a")] => Ok(Value::Bool(true)); "eq matches same strings")]
    #[test_case("eq", vec![s("1"), Value::Int(1)] => Ok(Value::Bool(false)); "eq never matches across types")]
    #[test_case("ne", vec![s("a"), s("b")] => Ok(Value::Bool(true)); "ne distinguishes strings")]
    #[test_case("gt", vec![Value::Int(3), Value::Int(2)] => Ok(Value::Bool(true)); "gt compares ints")]
    #[test_case("gte", vec![Value::Int(2), Value::Int(2)] => Ok(Value::Bool(true)); "gte allows equal ints")]
    #[test_case("lt", vec![Value::Int(2), Value::Int(3)] => Ok(Value::Bool(true)); "lt compares ints")]
    #[test_case("lte", vec![Value::Int(2), Value::Int(2)] => Ok(Value::Bool(true)); "lte allows equal ints")]
    #[test_case("add", vec![Value::Int(1), Value::Int(2), Value::Int(3)] => Ok(Value::Int(6)); "add folds variadic ints")]
    #[test_case("sub", vec![Value::Int(5), Value::Int(2)] => Ok(Value::Int(3)); "sub subtracts ints")]
    #[test_case("mul", vec![Value::Int(2), Value::Int(3), Value::Int(4)] => Ok(Value::Int(24)); "mul folds variadic ints")]
    #[test_case("div", vec![Value::Int(7), Value::Int(2)] => Ok(Value::Int(3)); "div truncates toward zero")]
    #[test_case("neg", vec![Value::Int(5)] => Ok(Value::Int(-5)); "neg negates an int")]
    #[test_case("upper", vec![s("aBc")] => Ok(s("ABC")); "upper uppercases a string")]
    #[test_case("lower", vec![s("aBc")] => Ok(s("abc")); "lower lowercases a string")]
    #[test_case("trim", vec![s("  x  ")] => Ok(s("x")); "trim strips surrounding whitespace")]
    #[test_case("replace", vec![s("aXa"), s("X"), s("Y")] => Ok(s("aYa")); "replace swaps all occurrences")]
    #[test_case("split", vec![s("a,,b"), s(",")] => Ok(list(vec![s("a"), s(""), s("b")])); "split keeps empty parts")]
    #[test_case("join", vec![s(", "), s("a"), s("b")] => Ok(s("a, b")); "join interleaves the separator")]
    #[test_case("join", vec![s("-"), s("only")] => Ok(s("only")); "join of one part needs no separator")]
    #[test_case("starts_with", vec![s("abc"), s("ab")] => Ok(Value::Bool(true)); "starts_with matches a prefix")]
    #[test_case("ends_with", vec![s("abc"), s("ab")] => Ok(Value::Bool(false)); "ends_with rejects a non-suffix")]
    #[test_case("not", vec![Value::Bool(true)] => Ok(Value::Bool(false)); "not negates a bool")]
    #[test_case("and", vec![Value::Bool(true), Value::Bool(false)] => Ok(Value::Bool(false)); "and requires all true")]
    #[test_case("or", vec![Value::Bool(false), Value::Bool(true)] => Ok(Value::Bool(true)); "or passes on any true")]
    #[test_case("is_truthy", vec![s("")] => Ok(Value::Bool(false)); "is_truthy rejects an empty string")]
    #[test_case("is_truthy", vec![Value::Int(0)] => Ok(Value::Bool(false)); "is_truthy rejects zero")]
    #[test_case("is_truthy", vec![Value::Bool(true)] => Ok(Value::Bool(true)); "is_truthy passes a true bool")]
    #[test_case("is_truthy", vec![list(vec![])] => Ok(Value::Bool(false)); "is_truthy rejects an empty list")]
    #[test_case("is_truthy", vec![list(vec![Value::Int(0)])] => Ok(Value::Bool(true)); "is_truthy passes a nonempty list")]
    #[test_case("coalesce", vec![s(""), s("fb"), s("x")] => Ok(s("fb")); "coalesce returns the first truthy")]
    #[test_case("coalesce", vec![s(""), s("")] => Ok(s("")); "coalesce falls back to the last arg")]
    #[test_case("to_str", vec![Value::Int(-3)] => Ok(s("-3")); "to_str stringifies an int")]
    #[test_case("to_str", vec![Value::Bool(true)] => Ok(s("true")); "to_str stringifies a bool")]
    #[test_case("to_str", vec![list(vec![Value::Int(1), s("a")])] => Ok(s("[1, a]")); "to_str stringifies a list")]
    #[test_case("to_str", vec![map(&[("k", s("v"))])] => Ok(s("{k: v}")); "to_str stringifies a map")]
    #[test_case("to_int", vec![Value::Int(3)] => Ok(Value::Int(3)); "to_int keeps ints")]
    #[test_case("to_int", vec![Value::Bool(true)] => Ok(Value::Int(1)); "to_int maps true to one")]
    #[test_case("to_int", vec![s("42")] => Ok(Value::Int(42)); "to_int parses decimal strings")]
    #[test_case("length", vec![s("abc")] => Ok(Value::Int(3)); "length counts string bytes")]
    #[test_case("length", vec![list(vec![Value::Int(1), Value::Int(2)])] => Ok(Value::Int(2)); "length counts list items")]
    #[test_case("length", vec![map(&[("a", Value::Int(1)), ("b", Value::Int(2))])] => Ok(Value::Int(2)); "length counts map entries")]
    #[test_case("contains", vec![s("abc"), s("b")] => Ok(Value::Bool(true)); "contains searches strings")]
    #[test_case("contains", vec![list(vec![s("a")]), s("a")] => Ok(Value::Bool(true)); "contains searches lists")]
    #[test_case("contains", vec![list(vec![s("a")]), s("b")] => Ok(Value::Bool(false)); "contains reports missing list items")]
    #[test_case("first", vec![list(vec![s("a"), s("b")])] => Ok(s("a")); "first returns the head")]
    #[test_case("last", vec![list(vec![s("a"), s("b")])] => Ok(s("b")); "last returns the tail")]
    #[test_case("keys", vec![map(&[("b", Value::Int(2)), ("a", Value::Int(1))])] => Ok(list(vec![s("a"), s("b")])); "keys lists sorted keys")]
    #[test_case("values", vec![map(&[("b", Value::Int(2)), ("a", Value::Int(1))])] => Ok(list(vec![Value::Int(1), Value::Int(2)])); "values lists values in key order")]
    fn evaluates_functions(name: &str, args: Vec<Value>) -> Result<Value, RegistryError> {
        call(name, &args)
    }

    #[test_case("upper", vec![]; "upper rejects zero args")]
    #[test_case("upper", vec![s("x"), s("y")]; "upper rejects two args")]
    #[test_case("os", vec![s("x")]; "os rejects an arg")]
    #[test_case("env", vec![s("x")]; "env rejects one arg")]
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
    #[test_case("replace", vec![Value::Int(1), s("x"), s("y")]; "replace rejects an int receiver")]
    #[test_case("split", vec![s("x"), Value::Int(1)]; "split rejects an int separator")]
    #[test_case("starts_with", vec![Value::Int(1), s("x")]; "starts_with rejects an int receiver")]
    #[test_case("ends_with", vec![s("x"), Value::Int(1)]; "ends_with rejects an int suffix")]
    #[test_case("gt", vec![s("1"), Value::Int(2)]; "gt rejects a string")]
    #[test_case("add", vec![Value::Int(1), s("2")]; "add rejects a string operand")]
    #[test_case("div", vec![s("1"), Value::Int(2)]; "div rejects a string operand")]
    #[test_case("neg", vec![s("1")]; "neg rejects a string")]
    #[test_case("not", vec![Value::Int(1)]; "not rejects an int")]
    #[test_case("and", vec![Value::Bool(true), Value::Int(1)]; "and rejects an int")]
    #[test_case("or", vec![Value::Int(0), Value::Bool(true)]; "or rejects an int")]
    #[test_case("first", vec![s("x")]; "first rejects a string")]
    #[test_case("last", vec![Value::Int(3)]; "last rejects an int")]
    #[test_case("keys", vec![list(vec![])]; "keys rejects a list")]
    #[test_case("values", vec![Value::Int(1)]; "values rejects an int")]
    #[test_case("enumerate", vec![s("x")]; "enumerate rejects a string")]
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
        let error = call("to_int", &[s("12x")]).unwrap_err();
        assert_eq!(
            error,
            RegistryError::Custom {
                msg: "cannot convert \"12x\" to Int".into(),
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
                msg: "first of an empty list".into(),
                indexes: vec![]
            }
        );
        assert_eq!(
            last,
            RegistryError::Custom {
                msg: "last of an empty list".into(),
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
        let error = call("contains", &[Value::Int(1), s("x")]).unwrap_err();
        assert!(matches!(error, RegistryError::Custom { indexes, .. } if indexes == vec![0]));
    }

    #[test]
    fn enumerate_pairs_indices_with_uniform_value_maps() {
        let result = call("enumerate", &[list(vec![s("a"), Value::Int(1)])]).unwrap();
        let expected = list(vec![
            map(&[("index", Value::Int(0)), ("value", s("a"))]),
            map(&[("index", Value::Int(1)), ("value", Value::Int(1))]),
        ]);
        assert_eq!(result, expected);
    }

    #[test]
    fn env_reads_the_process_environment() {
        let value = env::var("HOME").expect("HOME must be set for tests");
        assert_eq!(call("env", &[s("HOME"), s("fb")]).unwrap(), s(&value));
    }

    #[test]
    fn env_falls_back_when_unset() {
        assert_eq!(
            call("env", &[s("DOTRIFT_TEST_BUILTIN_MISSING"), s("fb")]).unwrap(),
            s("fb")
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
