use std::collections::HashMap;

use templater::{FuncError, Value, ValueType, function::FunctionRegistry};

macro_rules! arg_count {
    ($args:ident, $name:expr, 0) => {
        if !$args.is_empty() {
            return Err(FuncError::WrongArgCount {
                name: $name.into(),
                expected: "0".into(),
                got: $args.len(),
            });
        }
    };
    ($args:ident, $name:expr, $n:literal) => {
        if $args.len() != $n {
            return Err(FuncError::WrongArgCount {
                name: $name.into(),
                expected: stringify!($n).into(),
                got: $args.len(),
            });
        }
    };
}

macro_rules! cast {
    ($e:expr, $t:ident, $i:literal) => {{
        let val = $e;
        match val {
            Value::$t(s) => s,
            _ => {
                return Err(FuncError::TypeMismatch {
                    arg: Some($i),
                    expected: ValueType::from(val).type_name(),
                    got: val.type_name(),
                });
            }
        }
    }};
}

macro_rules! register_fn {
    ($map:ident, $name:literal, $n:literal, | $args:ident | $body:expr) => {
        $map.insert(
            $name.into(),
            Box::new(|$args| {
                arg_count!($args, $name, $n);
                $body
            }),
        );
    };
}

type FnEntry = Box<dyn Fn(&[Value]) -> Result<Value, FuncError>>;

pub struct BuiltinFunctions {
    functions: HashMap<String, FnEntry>,
}

fn truthy(val: &Value) -> bool {
    match val {
        Value::Str(s) => !s.is_empty(),
        Value::Int(n) => *n != 0,
        Value::Bool(b) => *b,
        Value::List(l) => !l.is_empty(),
        Value::Map(m) => !m.is_empty(),
    }
}

impl BuiltinFunctions {
    pub fn new() -> Self {
        let mut f: HashMap<String, FnEntry> = HashMap::new();

        register_fn!(f, "env", 2, |args| {
            let var = cast!(&args[0], Str, 0);
            let fallback = cast!(&args[1], Str, 1);
            Ok(Value::Str(
                std::env::var(var).unwrap_or_else(|_| fallback.clone()),
            ))
        });

        register_fn!(f, "home", 0, |args| {
            let home = std::env::var("HOME")
                .ok()
                .or_else(|| dirs::home_dir().map(|p| p.to_string_lossy().into_owned()))
                .unwrap_or_default();
            Ok(Value::Str(home))
        });

        register_fn!(f, "os", 0, |args| {
            Ok(Value::Str(std::env::consts::OS.into()))
        });

        register_fn!(f, "arch", 0, |args| {
            Ok(Value::Str(std::env::consts::ARCH.into()))
        });

        f.insert(
            "join".into(),
            Box::new(|args| {
                if args.len() < 2 {
                    return Err(FuncError::WrongArgCount {
                        name: "join".into(),
                        expected: "2+".into(),
                        got: args.len(),
                    });
                }
                let sep = cast!(&args[0], Str, 0);
                let mut result = String::new();
                for (i, arg) in args[1..].iter().enumerate() {
                    let Value::Str(s) = arg else {
                        return Err(FuncError::TypeMismatch {
                            arg: Some(i + 1),
                            expected: "String",
                            got: arg.type_name(),
                        });
                    };
                    if i > 0 {
                        result.push_str(sep);
                    }
                    result.push_str(s);
                }
                Ok(Value::Str(result))
            }),
        );

        register_fn!(f, "upper", 1, |args| {
            let s = cast!(&args[0], Str, 0);
            Ok(Value::Str(s.to_uppercase()))
        });

        register_fn!(f, "lower", 1, |args| {
            let s = cast!(&args[0], Str, 0);
            Ok(Value::Str(s.to_lowercase()))
        });

        register_fn!(f, "replace", 3, |args| {
            let s = cast!(&args[0], Str, 0);
            let from = cast!(&args[1], Str, 1);
            let to = cast!(&args[2], Str, 2);
            Ok(Value::Str(s.replace(from.as_str(), to.as_str())))
        });

        register_fn!(f, "trim", 1, |args| {
            let s = cast!(&args[0], Str, 0);
            Ok(Value::Str(s.trim().to_string()))
        });

        f.insert(
            "coalesce".into(),
            Box::new(|args| {
                if args.is_empty() {
                    return Err(FuncError::WrongArgCount {
                        name: "coalesce".into(),
                        expected: "1+".into(),
                        got: args.len(),
                    });
                }
                for arg in args {
                    if truthy(arg) {
                        return Ok(arg.clone());
                    }
                }
                Ok(args.last().unwrap().clone())
            }),
        );

        register_fn!(f, "length", 1, |args| {
            match &args[0] {
                Value::List(l) => Ok(Value::Int(l.len() as i64)),
                Value::Map(m) => Ok(Value::Int(m.len() as i64)),
                Value::Str(s) => Ok(Value::Int(s.len() as i64)),
                _ => Err(FuncError::TypeMismatch {
                    arg: Some(0),
                    expected: "List|Map|String",
                    got: args[0].type_name(),
                }),
            }
        });

        f.insert(
            "contains".into(),
            Box::new(|args| {
                if args.len() != 2 {
                    return Err(FuncError::WrongArgCount {
                        name: "contains".into(),
                        expected: "2".into(),
                        got: args.len(),
                    });
                }
                let found = match &args[0] {
                    Value::List(l) => l.contains(&args[1]),
                    Value::Map(m) => {
                        let Value::Str(k) = &args[1] else {
                            return Err(FuncError::TypeMismatch {
                                arg: Some(1),
                                expected: "String",
                                got: args[1].type_name(),
                            });
                        };
                        m.contains_key(k)
                    }
                    _ => {
                        return Err(FuncError::TypeMismatch {
                            arg: Some(0),
                            expected: "List|Map",
                            got: args[0].type_name(),
                        });
                    }
                };
                Ok(Value::Bool(found))
            }),
        );

        Self { functions: f }
    }
}

impl FunctionRegistry for BuiltinFunctions {
    fn call(&self, name: &str, args: &[Value]) -> Result<Value, FuncError> {
        self.functions
            .get(name)
            .map(|f| f(args))
            .unwrap_or_else(|| Err(FuncError::Undefined(name.into())))
    }
}

impl Default for BuiltinFunctions {
    fn default() -> Self {
        Self::new()
    }
}
