# 02 — Interpolation of String/Int/Bool variables and literals

**What to build:** `{{ name }}`, `{{ "literal" }}`, `{{ 42 }}`, `{{ -7 }}`, and `{{ true }}` all parse and render. The scanner recognizes `{{ }}` only (no comments, no statements, no modifiers, no escapes yet). The parser builds `Node::Interpolate(Expr::Var | StrLit | IntLit | BoolLit)`; integer literals decode at parse time to `i64`, booleans to `bool`, string literals stay as a raw byte `Range<usize>` and are escape-walked directly into the writer on render (zero allocation). The evaluator walks the `Frame::Var(&HashMap<String, Value>)` base scope (no loops yet), returns an owned `Value` per lookup (cloning under decision E1), and `Value::write_top` emits String verbatim, Int in decimal with leading `-` for negatives, Bool as `true`/`false`. Parse-time errors fire on empty interpolation `{{}}`, integer out-of-i64-range, and `+7`. Render-time error fires on undefined variable.

**Blocked by:** 01

**Status:** ready-for-agent

- [x] Scanner recognizes `{{` and `}}` delimiters, trims inner-edge whitespace, emits `Token::Text` and `Token::Interp(body_range)` configurations; escapes, modifiers, and the other four delimiters still inert at this slice.
- [x] Parser produces `Node::Interpolate(Expr)` with `Expr::{Var(Range<usize>), StrLit(Range<usize>), IntLit(i64), BoolLit(bool)}`; AST carries zero owned `String`s.
- [x] `eval` walks `Frame::Var(&HashMap)` base scope, returns owned `Value`; `lookup` returns `Cow<Value>` (`Borrowed` from the Var frame).
- [x] `Value::write_top` emits Str verbatim, Int decimal with leading `-` for negatives, Bool `true`|`false`.
- [x] String literals decode on render via escape-walk straight into the writer; `\"` and `\\` recognized, other `\X` pass through both bytes verbatim; raw newlines inside string literals preserved.
- [x] Parse errors: empty interpolation body, integer out-of-i64-range, `+7`-prefixed integer.
- [x] Render error: undefined variable carries the variable-name byte span.
- [x] `tests/common/mod.rs` lands minimal helpers (a `render` wrapper and `MockRegistry` — `TestRegistry` added in ticket 04).
- [x] End-to-end tests in `templater/tests/render.rs` cover each literal/var interpolation shape; error tests in `templater/tests/error.rs` cover parse and undefined-variable cases via `matches!` + byte span equality.
- [x] `cargo test -p templater`, `cargo fmt`, `cargo clippy -p templater` pass.
