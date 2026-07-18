# 04 — Function calls

**What to build:** `name(arg, arg, ...)`, `name()` (zero args), and nested calls like `eq(a, b)` and `join(":", home(), ".bin")` parse, evaluate, and render the returned value. The parser disallows trailing commas in argument lists and reserved keywords (`if`/`elif`/`else`/`for`/`in`/`end`) as function names (`if()`, `1st()`, `kebab-fn()` are parse errors). The evaluator builds an owned `Vec<Value>` of args by calling `eval` per argument, dispatches through `&dyn FunctionRegistry::call(name, &args)`, and lifts the returned `FuncError` into `Error::Func` attributing the `FnCall.name` byte span. `FuncError` has variants `Undefined`, `ArgCount { expected, got }`, and `TypeMismatch` (structured per render-time type cases). `templater/tests/common/mod.rs` lands the `TestRegistry` exposing a small set (`eq`, `not`, `length`, `join`) so subsequent tickets and this one's tests can exercise real function calls.

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] `Expr::FnCall { name: Range<usize>, args: Vec<Expr> }` parses; nested calls supported; zero-arg calls supported.
- [ ] Parser rejects trailing commas in argument lists and reserved keywords as function names.
- [ ] `FunctionRegistry::call(&self, name: &str, args: &[Value]) -> Result<Value, FuncError>` is the host trait; name extracted via `std::str::from_utf8` (parser guarantees ASCII identifier grammar).
- [ ] Eval builds `Vec<Value>` per call via `eval` per arg; passes `&args` to the registry; returns the owned `Value` for further interpolation or nesting.
- [ ] `FuncError` variants (`Undefined`, `ArgCount`, `TypeMismatch`) defined; lifted to `Error::Func` at the call boundary attributing `FnCall.name` byte span.
- [ ] `tests/common/mod.rs` lands `MockRegistry` (always `Undefined`) and `TestRegistry` (`eq`, `not`, `length`, `join`) for use by this and later tickets.
- [ ] End-to-end tests in `templater/tests/render.rs` cover happy-path nested calls; error tests in `templater/tests/error.rs` cover undefined function, wrong arg count, type mismatch with `matches!` + byte span equality.
- [ ] `cargo test -p templater`, `cargo fmt`, `cargo clippy -p templater` pass.