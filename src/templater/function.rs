use std::collections::{BTreeMap, HashMap};

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

        register_fn!(f, "split", 2, |args| {
            let s = cast!(&args[0], Str, 0);
            let sep = cast!(&args[1], Str, 1);
            Ok(Value::List(
                s.split(sep.as_str())
                    .map(|p| Value::Str(p.to_string()))
                    .collect(),
            ))
        });

        register_fn!(f, "starts_with", 2, |args| {
            let s = cast!(&args[0], Str, 0);
            let prefix = cast!(&args[1], Str, 1);
            Ok(Value::Bool(s.starts_with(prefix.as_str())))
        });

        register_fn!(f, "ends_with", 2, |args| {
            let s = cast!(&args[0], Str, 0);
            let suffix = cast!(&args[1], Str, 1);
            Ok(Value::Bool(s.ends_with(suffix.as_str())))
        });

        register_fn!(f, "eq", 2, |args| Ok(Value::Bool(args[0] == args[1])));

        register_fn!(f, "ne", 2, |args| Ok(Value::Bool(args[0] != args[1])));

        register_fn!(f, "gt", 2, |args| {
            let a = cast!(&args[0], Int, 0);
            let b = cast!(&args[1], Int, 1);
            Ok(Value::Bool(a > b))
        });

        register_fn!(f, "gte", 2, |args| {
            let a = cast!(&args[0], Int, 0);
            let b = cast!(&args[1], Int, 1);
            Ok(Value::Bool(a >= b))
        });

        register_fn!(f, "lt", 2, |args| {
            let a = cast!(&args[0], Int, 0);
            let b = cast!(&args[1], Int, 1);
            Ok(Value::Bool(a < b))
        });

        register_fn!(f, "lte", 2, |args| {
            let a = cast!(&args[0], Int, 0);
            let b = cast!(&args[1], Int, 1);
            Ok(Value::Bool(a <= b))
        });

        f.insert(
            "add".into(),
            Box::new(|args| {
                if args.len() < 2 {
                    return Err(FuncError::WrongArgCount {
                        name: "add".into(),
                        expected: "2+".into(),
                        got: args.len(),
                    });
                }
                let mut sum = 0i64;
                for arg in args {
                    let Value::Int(n) = arg else {
                        return Err(FuncError::TypeMismatch {
                            arg: None,
                            expected: "Int",
                            got: arg.type_name(),
                        });
                    };
                    sum += n;
                }
                Ok(Value::Int(sum))
            }),
        );

        register_fn!(f, "sub", 2, |args| {
            let a = cast!(&args[0], Int, 0);
            let b = cast!(&args[1], Int, 1);
            Ok(Value::Int(a - b))
        });

        f.insert(
            "mul".into(),
            Box::new(|args| {
                if args.len() < 2 {
                    return Err(FuncError::WrongArgCount {
                        name: "mul".into(),
                        expected: "2+".into(),
                        got: args.len(),
                    });
                }
                let mut product = 1i64;
                for arg in args {
                    let Value::Int(n) = arg else {
                        return Err(FuncError::TypeMismatch {
                            arg: None,
                            expected: "Int",
                            got: arg.type_name(),
                        });
                    };
                    product *= n;
                }
                Ok(Value::Int(product))
            }),
        );

        register_fn!(f, "div", 2, |args| {
            let a = cast!(&args[0], Int, 0);
            let b = cast!(&args[1], Int, 1);
            if *b == 0 {
                return Err(FuncError::Custom("division by zero".into()));
            }
            Ok(Value::Int(a / b))
        });

        register_fn!(f, "neg", 1, |args| {
            let a = cast!(&args[0], Int, 0);
            Ok(Value::Int(-a))
        });

        f.insert(
            "and".into(),
            Box::new(|args| {
                if args.len() < 2 {
                    return Err(FuncError::WrongArgCount {
                        name: "and".into(),
                        expected: "2+".into(),
                        got: args.len(),
                    });
                }
                for arg in args {
                    let Value::Bool(b) = arg else {
                        return Err(FuncError::TypeMismatch {
                            arg: None,
                            expected: "Bool",
                            got: arg.type_name(),
                        });
                    };
                    if !b {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(true))
            }),
        );

        f.insert(
            "or".into(),
            Box::new(|args| {
                if args.len() < 2 {
                    return Err(FuncError::WrongArgCount {
                        name: "or".into(),
                        expected: "2+".into(),
                        got: args.len(),
                    });
                }
                for arg in args {
                    let Value::Bool(b) = arg else {
                        return Err(FuncError::TypeMismatch {
                            arg: None,
                            expected: "Bool",
                            got: arg.type_name(),
                        });
                    };
                    if *b {
                        return Ok(Value::Bool(true));
                    }
                }
                Ok(Value::Bool(false))
            }),
        );

        register_fn!(f, "not", 1, |args| {
            let b = cast!(&args[0], Bool, 0);
            Ok(Value::Bool(!b))
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

        register_fn!(f, "to_str", 1, |args| {
            fn val_str(v: &Value) -> String {
                match v {
                    Value::Str(s) => s.clone(),
                    Value::Int(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    Value::List(l) => {
                        let items: Vec<String> = l.iter().map(val_str).collect();
                        format!("[{}]", items.join(", "))
                    }
                    Value::Map(m) => {
                        let items: Vec<String> = m
                            .iter()
                            .map(|(k, v)| format!("{k}: {}", val_str(v)))
                            .collect();
                        format!("{{{}}}", items.join(", "))
                    }
                }
            }
            Ok(Value::Str(val_str(&args[0])))
        });

        register_fn!(f, "to_int", 1, |args| {
            match &args[0] {
                Value::Int(n) => Ok(Value::Int(*n)),
                Value::Bool(b) => Ok(Value::Int(if *b { 1 } else { 0 })),
                Value::Str(s) => s
                    .parse::<i64>()
                    .map(Value::Int)
                    .map_err(|_| FuncError::Custom(format!("cannot convert \"{s}\" to Int"))),
                _ => Err(FuncError::TypeMismatch {
                    arg: Some(0),
                    expected: "Int|Bool|String",
                    got: args[0].type_name(),
                }),
            }
        });

        register_fn!(f, "is_truthy", 1, |args| {
            Ok(Value::Bool(truthy(&args[0])))
        });

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

        register_fn!(f, "contains", 2, |args| {
            let found = match &args[0] {
                Value::Str(s) => s.contains(cast!(&args[1], Str, 1)),
                Value::List(l) => l.contains(&args[1]),
                Value::Map(m) => {
                    let k = cast!(&args[1], Str, 1);
                    m.contains_key(k)
                }
                _ => {
                    return Err(FuncError::TypeMismatch {
                        arg: Some(0),
                        expected: "String|List|Map",
                        got: args[0].type_name(),
                    });
                }
            };
            Ok(Value::Bool(found))
        });

        register_fn!(f, "first", 1, |args| {
            let list = cast!(&args[0], List, 0);
            list.first()
                .cloned()
                .ok_or_else(|| FuncError::Custom("first: empty list".into()))
        });

        register_fn!(f, "last", 1, |args| {
            let list = cast!(&args[0], List, 0);
            list.last()
                .cloned()
                .ok_or_else(|| FuncError::Custom("last: empty list".into()))
        });

        register_fn!(f, "keys", 1, |args| {
            let map = cast!(&args[0], Map, 0);
            Ok(Value::List(
                map.keys().map(|k| Value::Str(k.clone())).collect(),
            ))
        });

        register_fn!(f, "values", 1, |args| {
            let map = cast!(&args[0], Map, 0);
            Ok(Value::List(map.values().cloned().collect()))
        });

        register_fn!(f, "enumerate", 1, |args| {
            let list = cast!(&args[0], List, 0);
            let result: Vec<Value> = list
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let mut map = match v {
                        Value::Map(m) => m.clone(),
                        _ => {
                            let mut m = BTreeMap::new();
                            m.insert("value".to_string(), v.clone());
                            m
                        }
                    };
                    map.insert("index".to_string(), Value::Int(i as i64));
                    Value::Map(map)
                })
                .collect();
            Ok(Value::List(result))
        });

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
