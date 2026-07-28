# 06 — Control-flow blocks: `if`/`elif`/`else`/`for`/`end`

**What to build:** All statement tags parse via recursive descent over the token stream and render correctly. `{% if expr %}` / `{% elif expr %}` / `{% else %}` / `{% end %}` blocks short-circuit in source order — the first `Bool(true)` arm renders; non-Bool condition is a render-time error; branches not taken don't fire render errors inside their bodies (so `{{ undefined_var }}` inside `{% if false %}` is silent). `{% for var in expr %}` / `{% end %}` blocks evaluate the iterable once at loop entry via `eval`, capture an owned `Value::List(Vec<Value>)`, and consume it via `into_iter()`; each iteration pushes `Frame::Loop { name: Range<usize>, value }`, renders the body, then pops. The loop variable shadows outer bindings inside the body and is restored at `{% end %}` (the stack pops naturally); an empty iterable skips the body entirely and never pushes a `Loop` frame, preserving the outer binding. Iterables may be variables, function calls, list literals, or dot-access expressions. `if`/`elif`/`else` introduce no new scope; only `for` does. Parser disallows `endif`/`endfor` (unrecognized statements), `else if` (else takes no operand), `{%ifx%}` (keyword must be a complete identifier, not `if x`); unclosed `{% if %}` or `{% for %}` and orphan `{% end %}` are parse errors. `for` parser requires an identifier as loop variable (keywords as loop var are parse errors), then `in`, then an iterable expression; trailing tokens after the iterable are parse errors; malformed binding (e.g. `obj.field` as loop var) is a parse error. Render error: non-List iterable carries the iterable-expression byte span.

**Blocked by:** 02, 04, 05

**Status:** ready-for-agent

- [x] `Node::If { arms: Vec<Arm>, else_body: Option<Vec<Node>> }` parses via `parse_block_body(stop_at: Some(Elif|Else|End))`; arms carry `cond: Expr` and `body: Vec<Node>`.
- [x] `Node::For { var: Range<usize>, iter: Expr, body: Vec<Node> }` parses; loop-var identifier is keyword-checked.
- [x] `{% end %}` matches the innermost unclosed block via the recursive-descent call stack (LIFO); unclosed openers and orphan `{% end %}` are parse errors.
- [x] Parser disallows `endif`/`endfor`, `else if`, `{%ifx%}` (longest-identifier match against reserved-keyword set), trailing tokens in `for`, missing `in`, malformed binding.
- [x] Eval short-circuits `if` arms in source order; non-Bool condition raises `RenderError::NonBoolCondition` with the condition-expression byte span; un-taken branches' bodies are never visited (render errors silently skip).
- [x] `for` evaluates the iterable once; non-List iterable raises `RenderError::NonListIterable` with the iterable-expression byte span.
- [x] `for` consumes the owned `Vec<Value>` via `into_iter()` (O(1) per iter, zero per-iter clones); pushes `Frame::Loop { name: Range<usize>, value }` per iteration; pops after body.
- [x] Loop variable shadows outer bindings for the body; outer binding is restored at `{% end %}` (stack-pop semantics); empty iterable skips body and preserves outer binding.
- [x] End-to-end tests in `templater/tests/render.rs` cover nested `if`/`elif`/`else`, `for` over each iterable shape (variable, list literal, function call, dot-access), nested `for` with shadowed loop names, empty iterable; error tests in `templater/tests/error.rs` cover non-Bool condition, non-List iterable, and each parse-time error variant.
- [x] `cargo test -p templater`, `cargo fmt`, `cargo clippy -p templater` pass.
