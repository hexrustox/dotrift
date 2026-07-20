# 05 — List literals, dot-access, aggregate canonical forms

**What to build:** `[a, b, c]` list literals (including `[]`) parse and evaluate to `Value::List(Vec<Value>)`; each element is evaluated via `eval` (cloning under E1). `obj.field` looks up a Map key by identifier; `list.0` indexes a List by non-negative integer. `Value::write_nested` renders List canonical forms (`[elem, elem]`, empty `[]`) and Map canonical forms (`{"key": value, ...}`, empty `{}`) with byte-level `\"` and `\\` escapes applied to nested string elements and string map keys (other backslash sequences pass through). String values are not re-escaped with respect to delimiters — a String containing `{{` emits those bytes raw. Map key ordering is unspecified-by-spec but deterministic via `BTreeMap`'s natural iteration. Parse-time errors: trailing comma in list literals, empty identifier after `.`. Render-time errors: dot-access `.identifier` on a non-Map value, list-index `.integer` on a non-List value, any dot-access on a String value, negative list index, list index out of bounds, map key not found.

**Blocked by:** 02

**Status:** done

- [x] `Expr::List(Vec<Expr>)` parses; empty `[]` accepted; trailing commas in list literals are parse errors; whitespace inside list brackets is optional and trimmed.
- [x] `Expr::Dot { left: Box<Expr>, field: Range<usize> }` performs Map key lookup on a Map receiver; empty identifier after `.` is a parse error.
- [x] `Expr::Index { left: Box<Expr>, idx: i64, idx_span: Range<usize> }` performs List index lookup; negative indices are stored and rejected at render time.
- [x] `Value::write_nested` renders List and Map canonical forms with byte-level `\"` and `\\` escapes on nested strings (no delimiter re-escaping); Map iteration order is `BTreeMap`'s natural order.
- [x] Render errors: dot-access `.field` on non-Map, `.integer` on non-List, dot-access on String, negative list index, list index out of bounds, map key not found — each carries the offending expression's byte span.
- [x] End-to-end tests in `templater/tests/render.rs` cover list-literal round-trips, nested aggregate canonical forms, dot-access chains; error tests in `templater/tests/error.rs` cover each render-time type error.
- [x] `cargo test -p templater`, `cargo fmt`, `cargo clippy -p templater` pass.