Status: ready-for-agent

# Spec: Templater engine

## Problem Statement

A dotfile manager needs to render templates — text documents with embedded tags — into final output by substituting variables, calling host-provided functions, and executing control flow. Today the dotrift workspace has no usable template engine: `templater/src/lib.rs` is an empty stub and `templater/legacy/` archives a prior implementation that is forbidden as a copy source. dotrift's `templater` CLI subcommand and its `apply`/`add`/`unapply` paths all depend on a working engine.

The host needs a standalone, template-engine crate that accepts source from a file or an in-memory byte buffer, evaluates a small expression language with no operator overloading, and streams rendered bytes into a caller-provided writer. The engine must strictly enforce the syntax contract in `templater/spec/syntax.md` and the vocabulary in `templater/spec/CONTEXT.md`, separating parse-time errors (malformed source) from render-time errors (evaluated but failing content) so that branches not taken never error.

## Solution

A standalone `templater` crate exposing a pre-parsed `Template` value:
- Construct from a file path (mmap-backed) or an owned byte buffer.
- Parse once at construction; hold the AST plus the source byte slice.
- Render to any `io::Write` by walking the AST with a host-provided variable map (borrowed) and function registry, streaming bytes straight to the writer.
- Surface structured parse-time, render-time, and function errors with byte-accurate source spans via miette.

The engine operates on raw bytes end-to-end (non-UTF-8 source tolerated), and the AST carries only byte ranges into the owned source slice — no owned strings in the AST. Runtime `Value`s are owned and cheap-clone. Variable resolution walks a stack of scope frames where only `for` introduces a new frame.

## User Stories

1. As a dotrift developer, I want a `Template` type that I can construct from a file path, so that I can render dotfile templates that live on disk.
2. As a dotrift developer, I want a `Template` type that I can construct from an owned `Vec<u8>`, so that I can render templates assembled in memory (e.g. from the `--string` CLI flag).
3. As a dotrift developer, I want construction to fail with a structured parse-time error, so that I can report malformed templates to the user with a byte-accurate source span.
4. As a dotrift developer, I want `Template::render` to take a generic writer, so that I can stream output to files, `Vec<u8>` buffers, or stdout without engine coupling.
5. As a dotrift developer, I want `Template::render` to take the variable map by borrow, so that the host can render the same template repeatedly with the same map without re-supplying it.
6. As a dotrift developer, I want `Template::render` to take a `&dyn FunctionRegistry`, so that the host can plug in any function set without the engine knowing the concrete type.
7. As a dotrift developer, I want `Template::render` to flush the writer on success, so that callers wrapping in `BufWriter` don't lose buffered bytes on a clean finish.
8. As a dotrift developer, I want `Template::render` to stop at the first render-time error and return a structured error, so that I can surface the failing span to the user.
9. As a dotrift developer, I want render errors to fire only on actually-executed content, so that templates with errors inside `{% if false %}` branches don't fail.
10. As a template author, I want to embed `{{ expr }}` interpolation tags, so that I can substitute variable values into output.
11. As a template author, I want to embed `{% if %}`/`{% elif %}`/`{% else %}`/`{% end %}` blocks, so that I can conditionally render content based on Bool values.
12. As a template author, I want to embed `{% for var in expr %}`/`{% end %}` blocks, so that I can iterate over a List and bind a loop variable per iteration.
13. As a template author, I want to embed `{# comment #}` tags, so that I can annotate templates without affecting output.
14. As a template author, I want leading/trailing inner whitespace inside tag delimiters to be trimmed, so that `{{ x }}` and `{{x}}` are equivalent.
15. As a template author, I want tags to span multiple physical lines, so that I can format long expressions readably.
16. As a template author, I want string literals inside tag bodies to shield delimiters, so that `"}}"` inside `{{ "..." }}` does not close the tag.
17. As a template author, I want string literals to support `\"` and `\\` escapes while passing other `\X` through verbatim, so that I can include quotes and backslashes in literal text.
18. As a template author, I want string literals to span multiple lines, so that I can write block strings in tag bodies.
19. As a template author, I want to escape any of the six delimiters with an odd number of preceding backslashes, so that I can emit literal `{{`/`}}`/`{%`/`%}`/`{#`/`#}` to output.
20. As a template author, I want even-count backslash runs to render as `n/2` literal backslashes and activate the delimiter, so that my ordinary text isn't accidentally interpreted as an escape.
21. As a template author, I want the `{{-`/`-}}`/`{%-`/`-%}` modifiers to trim adjacent spaces and tabs on their side, so that I can keep my source readable without spoiling output.
22. As a template author, I want the `{{=`/`=}}`/`{%=`/`=%}` modifiers to scan to a line boundary or another tag's delimiter, so that I can collapse template scaffolding lines out of the output.
23. As a template author, I want rightward `=` to eat through and include the terminating newline, so that the entire scaffolding line disappears.
24. As a template author, I want leftward `=` to stop before the newline and at any tag's closing delimiter, so that preceding text on the line is preserved when appropriate.
25. As a template author, I want all six delimiters to act as `=` barriers, so that two `=`-tags on the same line each clean only the region between them.
26. As a template author, I want `-` and `=` to be mutually exclusive on the same side of a tag, so that ambiguous modifier combinations are rejected at parse time.
27. As a template author, I want comment delimiters to act as `=` barriers but carry no modifiers, so that comments take part in whitespace control without producing output.
28. As a template author, I want `{% if x %}` to require `x` to evaluate to Bool, so that non-Bool conditions never silently coerce.
29. As a template author, I want `{% for x in xs %}` to require `xs` to evaluate to List, so that non-List iterables never silently coerce.
30. As a template author, I want the loop variable to shadow outer bindings inside the body and restore them at `{% end %}`, so that I can reuse variable names inside loops without polluting enclosing scope.
31. As a template author, I want an empty iterable to skip the loop body and preserve the outer binding, so that empty lists behave predictably.
32. As a template author, I want `if`/`elif`/`else` to introduce no new scope, so that branch bodies see the enclosing scope directly.
33. As a template author, I want to write dot-access `obj.field` for Map fields and `list.0` for List indices, so that I can traverse structured values.
34. As a template author, I want negative list indices to be a render-time error, so that out-of-range indices don't silently wrap.
35. As a template author, I want dot-access on a non-Map (`.field`) or non-List (`.integer`) receiver to be a render-time error, so that type mismatches surface clearly.
36. As a template author, I want index access on a String to be a render-time error, so that strings aren't silently indexed by byte.
37. As a template author, I want map keys that don't match the identifier grammar to require a host function, so that only valid identifiers are reachable via dot syntax.
38. As a template author, I want to call functions with `name(arg, arg)` syntax including zero-arg calls, so that I can integrate host-provided helpers.
39. As a template author, I want function calls to nest but not pipe-chain, so that call syntax stays unambiguous.
40. As a template author, I want list literal `[a, b, c]` syntax including the empty list, so that I can build Lists inline.
41. As a template author, I want integer literals to support an optional leading `-`, leading zeros, and full i64 range, so that I can write normal integer constants.
42. As a template author, I want `+`-prefixed integer literals to be a parse-time error, so that the lexer doesn't accept `+7` as a valid integer.
43. As a template author, I want reserved keywords (`if`, `elif`, `else`, `for`, `in`, `end`) to be rejected as identifiers, so that control-flow words can't be repurposed as variable or function names.
44. As a template author, I want `{%ifx%}` to be a parse error (not `if x`), so that keyword recognition is unambiguous.
45. As a template author, I want `{% endif %}` and `{% endfor %}` to be parse errors, so that only the bare `{% end %}` form closes blocks.
46. As a template author, I want trailing commas in list literals and function calls to be parse errors, so that the grammar stays uniform.
47. As a template author, I want unclosed `{% if %}` or `{% for %}` blocks to be parse errors, so that every opener has a matching `{% end %}`.
48. As a template author, I want orphan `{% end %}` with no matching opener to be a parse error, so that unbalanced control flow surfaces immediately.
49. As a template author, I want empty interpolations `{{}}` and empty statements `{% %}` to be parse errors, so that stray delimiters surface.
50. As a template author, I want empty identifiers after `.` to be parse errors, so that `obj.` doesn't silently parse.
51. As a template author, I want integer literals overflowing i64 to be parse errors, so that out-of-range constants surface at parse time.
52. As a template author, I want String interpolation at the top level to emit bytes verbatim, so that I don't get surprise quoting or escaping in my output.
53. As a template author, I want String values nested inside List or Map canonical forms to be double-quoted with `\"` and `\\` escapes, so that the canonical form round-trips.
54. As a template author, I want String values to not be re-escaped with respect to delimiters, so that a String containing `{{` emits those bytes raw even inside List/Map forms.
55. As a template author, I want Int values to render in decimal with a leading `-` for negatives, so that integer output is unambiguous.
56. As a template author, I want Bool values to render as `true` or `false`, so that boolean output is canonical.
57. As a template author, I want List values to render as `[]` for empty or `[elem, elem]` for non-empty, so that list output round-trips.
58. As a template author, I want Map values to render as `{}` for empty or `{"key": value, ...}` for non-empty, so that map output round-trips.
59. As a host author, I want a `Value` enum with `Str`, `Int`, `Bool`, `List`, `Map` variants, so that I can build typed values for variables and function returns.
60. As a host author, I want `Value` to derive `Debug`, `Clone`, `PartialEq`, and `Eq`, so that I can use it in tests and as a BTreeMap value.
61. As a host author, I want a `FunctionRegistry` trait with a `call(&self, name: &str, args: &[Value]) -> Result<Value, FuncError>` method, so that I can implement my function set however I like.
62. As a host author, I want undefined variable access to be a render-time error, so that misspelled variable names surface only when reached.
63. As a host author, I want undefined function calls to be render-time errors, so that the registry is consulted only when needed.
64. As a host author, I want wrong-arg-count and type-mismatch function errors to surface as `FuncError` variants, so that I can pattern-match on host errors.
65. As a host author, I want function-call errors to attribute to the function-name byte span, so that diagnostics point at the call site.
66. As a host author, I want the engine to tolerate non-UTF-8 source bytes, so that I can render binary-adjacent templates without preprocessing.
67. As a host author, I want miette diagnostics to render byte-accurate spans via a custom `SourceCode` impl, so that error underlines line up with the actual offending bytes.
68. As a host author, I want construction-time IO errors (file open, mmap) and render-time IO errors (write, flush) to surface as `Error::Io(io::Error)`, so that IO failures are uniformly reportable.
69. As a host author, I want the templater crate to depend only on `memmap2`, `miette`, `thiserror` (plus `test-case` as a dev-dep), so that the dep surface stays minimal.
70. As a host author, I want to re-render a parsed `Template` multiple times with different variable maps, so that I can cache parsed templates and amortize parse cost.
71. As a host author, I want the templater's public API to be `'static`/lifetime-free, so that I can store `Template` values in long-lived structs without lifetime plumbing.
72. As a host author, I want the templater's AST and scope frame types to be internal, so that the public surface stays small and stable.
73. As a maintainer, I want scanner edge cases (escape rule, `=`/`-` modifiers, multi-line tags, string-shielding) to be unit-tested at the `scan()` function level, so that lexer behaviour is pinned.
74. As a maintainer, I want parser edge cases (keyword reservation, empty tags, malformed statements, integer range, trailing commas, unclosed/orphan blocks) to be unit-tested at the `parse()` function level, so that AST construction is pinned.
75. As a maintainer, I want render canonical forms per `Value` type to be tested end-to-end via the public API, so that output behaviour is pinned.
76. As a maintainer, I want error scenarios to assert on `Error` variant via `matches!` plus byte span equality, so that error tests are stable across miette upgrades.
77. As a maintainer, I want the templater crate to honor the repo convention of colocated `#[cfg(test)] mod tests` in `src/` modules, so that internal-shape tests live next to the code.

## Implementation Decisions

### Public API surface

The crate exports `Template`, `Value`, `ValueType`, `FunctionRegistry`, `Error`, `ParseError`, `RenderError`, `FuncError`, and a `Result<T>` alias. Internal modules (`ast`, `error`, `eval`, `function`, `parser`, `scanner`, `value`) expose only what the public surface requires.

Construction:
- `Template::from_file<P: AsRef<Path>>(path: P) -> Result<Self>` — mmap the file. Mmap failure, file-open failure, or parse failure surfaces as `Error::Io` or `Error::Parse`.
- `Template::from_bytes(bytes: Vec<u8>) -> Result<Self>` — own the byte buffer.

Rendering:
- `Template::render<W: io::Write>(&self, writer: W, variables: &HashMap<String, Value>, functions: &dyn FunctionRegistry) -> Result<()>` — borrows self (cheap re-render), borrows variables (host keeps them), generic writer (caller manages buffering), flushes on success. The public API is lifetime-free; any internal lifetime required to borrow the variable map stays private to the engine.

### Backing source

Private two-variant enum held by `Template`:
- `Source::Mapped(Mmap)` for file-backed templates — zero-copy, OS-paged on demand.
- `Source::Owned(Vec<u8>)` for byte-backed templates.

Both yield `'static` `Template` values with no lifetime parameter in the public API. The `Source` enum is `pub(crate)`.

### AST shape (internal)

The AST is held in `Template.nodes: Vec<Node>` at the top level. `If` and `For` nodes own nested `Vec<Node>` bodies (true recursive-tree AST — no flat-arena-with-jump-indices). Every identifier, string-literal, field name, function-name, and variable-reference position is stored as a byte `Range<usize>` into the source slice; no `String`s exist in the AST.

Integer literals decode to `i64` and boolean literals to `bool` at parse time (these are fixed-size and their parse-time error semantics require decoding). String literals stay as raw `Range<usize>` and are decoded on render (escape-walked straight into the writer — zero allocation). List-index access stores a decoded `i64` (negative indices are a render-time error per spec, not parse-time). Dot-field access stores a `Range<usize>` for the field name.

### Value type

The runtime `Value` enum:
- `Value::Str(String)` — owned.
- `Value::Int(i64)`.
- `Value::Bool(bool)`.
- `Value::List(Vec<Value>)`.
- `Value::Map(BTreeMap<String, Value>)` — sorted, deterministic iteration order for snapshot/error fidelity.

Derived: `Debug, Clone, PartialEq, Eq`. No `Ord`, `Hash`, or `Deserialize`. No `TryFrom<toml::Value>` — the host owns toml→Value conversion, letting the templater drop its `serde` and `toml` dependencies.

Canonical-form rendering splits into two private methods:
- `write_top<W: io::Write>` — top-level interpolation output. Strings emitted verbatim, no quoting, no delimiter re-escaping.
- `write_nested<W: io::Write>` — used inside List/Map canonical forms. Strings double-quoted with only `\"` and `\\` escapes (other backslash sequences pass through). Bytes are written straight to the writer via a byte-level escape loop — no intermediate allocation.

Map canonical form uses sorted key order (BTreeMap's natural iteration).

### Function registry

```rust
pub trait FunctionRegistry {
    fn call(&self, name: &str, args: &[Value]) -> Result<Value, FuncError>;
}
```

- `&str` lookup name extracted from the AST name range at render time (`std::str::from_utf8` — parser guarantees ASCII identifier grammar).
- `&[Value]` borrowed args — the engine builds an owned `Vec<Value>` per call and borrows it for the duration of the call.
- Engine-defined `FuncError` (Undefined / ArgCount / TypeMismatch) returned by the host, lifted to the crate's `Error::Func` at the call boundary, attributing the function-name byte span.

The host constructs whatever `FunctionRegistry` impl they like and passes `&dyn FunctionRegistry` to `render`. The engine does not own or build a registry.

### Scope stack (internal)

The render call borrows the host's `&HashMap<String, Value>` for the base scope and threads a private `'a` lifetime through the internal `Frame` type:

```rust
enum Frame<'src> {
    Var(&'a HashMap<String, Value>),
    Loop { name: Range<usize>, value: Value },
}
```

- `Frame::Var` borrows the host's base scope — zero copy at render entry.
- `Frame::Loop` is pushed per `for`-iteration, holding a byte-range name (borrowing source) and an owned element `Value`. The `'src` lifetime stays internal to the engine — the public `render` signature is lifetime-free.
- Resolution walks `scopes.iter().rev()` and returns `Cow<Value>` — `Cow::Borrowed` when found in the base `Var` frame, `Cow::Owned(value.clone())` when found in a `Loop` frame (since `eval` always returns owned under E1, this matches eval cloning anyway). Outer bindings are never mutated; shadowing falls out naturally; the spec's "outer binding preserved across `{% for %}…{% end %}" rule is satisfied without special machinery.

### Expression evaluation

A single private `eval(&self, e: &Expr, scopes: &mut Vec<Frame>) -> Result<Value>` method, always returning an owned `Value`. Every variable lookup clones (one `String` clone for `Value::Str`, deep clone for aggregates). This trade-off was chosen for implementation simplicity and predictable cost: the borrow-fast-path savings are marginal at dotfile scale and complicate function-argument evaluation (which needs owned values anyway).

- Dot-access traverses `&Value` receivers where borrowing is natural (leftmost `Expr::Var` borrows from scope; subsequent `Dot`/`Index` steps borrow from the receiver). When a step needs an owned intermediate (e.g. `fn().field`), the path transitions to `Value::clone()` at that point.
- List literals evaluate each element via `eval` and pack into a fresh `Vec<Value>`.
- Function calls evaluate each arg via `eval`, build the args `Vec`, and dispatch through the `FunctionRegistry`.
- `if` arms short-circuit in source order. First `Value::Bool(true)` arm renders; non-Bool condition is a render-time error. Untaken branches are never visited, so render errors inside them don't fire.

### `for` loop semantics

- Evaluate the iterable once at loop entry via `eval`; capture an owned `Value::List(Vec<Value>)`.
- If the iterable isn't `Value::List`, raise `RenderError::NonListIterable` with the iterable-expression byte span.
- Consume the owned `Vec<Value>` via `into_iter()` — O(1) per iteration, zero per-iter element clones. Iterables sourced from a variable pay one container-level clone at entry (unavoidable since `eval` clones on var lookup); list-literal and function-call iterables are already owned and pay zero.
- Each iteration pushes `Frame::Loop { name, value: element }`, renders the body, then pops the frame.
- Empty iterables skip the body entirely and never push a `Loop` frame — outer binding preserved naturally.

### Render loop (internal)

Recursive descent over the top-level `&self.nodes` slice and nested body slices:
```rust
fn eval_body(&self, range: &[Node], scopes: &mut Vec<Frame>, w: &mut W, fns: &dyn FunctionRegistry) -> Result<()>;
```
- `Node::Text(r)` → `w.write_all(&self.src[r])` (caller's `BufWriter` coalesces).
- `Node::Interpolate(e)` → `eval(e) -> Value` → `value.write_top(w)`.
- `Node::If` → short-circuit arms as above.
- `Node::For` → iterable consume-loop as above.
- Stops at the first error and returns. Flushes the writer on success.

### Scanner (internal)

Single-pass byte scan producing a flat `Vec<Token>`:
```rust
enum Token { Text(Range<usize>), Interp(Range<usize>), Stmt(Range<usize>) }
```
- Comments are stripped at scan time (no `Token::Comment` variant), but act as `=` barriers during scanning.
- Tag body ranges exclude inner-edge whitespace (already trimmed by the scanner — the parser doesn't re-trim).
- Escape rule: odd-backslash-run immediately before a delimiter renders it as literal text; even-run renders `n/2` literal backslashes and activates the delimiter. Applied inline during scan; no separate pass.
- String-literal shielding: inside interp/stmt tags, the scanner recognizes `"` and enters string-literal mode (scanning to the next unescaped `"`, with `\\` per the string escape rule) so `}}`/`%}` inside a string literal does not close the tag. Inside comments, no string-literal mode — bytes scan raw until unescaped `#}`.
- Multi-line tags: a tag body may span physical lines; raw newlines inside a string literal inside a tag body are preserved as part of the literal's value.
- Delimiter disambiguation: single-byte lookahead (`{` followed by `{`/`%`/`#` opens; otherwise plain text).
- Whitespace modifiers (`-`, `=`) applied inline during scan; emitted tokens carry no modifier info, only already-trimmed Text ranges. `=` leftward/rightward scans are over **raw bytes** (not over the token stream): they walk back/forward from the tag's opening/closing delimiter position, stopping at `\n`/SOF/EOF or another tag's delimiter (any of the six), per spec § Whitespace Control. Rightward `=` eats through and includes the terminating `\n`; leftward `=` stops before the `\n`. `\r` is plain text, not a line terminator.
- `-` trim is local: only adjacent spaces and tabs on its side, no `\n`, no cross-tag.
- `=` and `-` on the same side of a tag, or any modifier on a comment delimiter, is a parse-time error raised by the scanner.

### Parser (internal)

Recursive descent over `Vec<Token>`, appending to a `Vec<Node>` arena top-level. Per-block bodies are owned `Vec<Node>` (recursive assembly).
- `parse_block_body(stop_at: Option<Kw>)` returns a `Vec<Node>` for the body between tags; the stop keyword is consumed by the caller.
- `parse_if` / `parse_for` recursively assemble arms and bodies.
- Statement keyword recognition: full identifier reading (longest match), then comparison against the reserved-keyword set. `{%ifx%}` reads `ifx`, not `if x` — parse error. `endif`/`endfor` not recognized — parse error.
- `for` parser requires identifier-as-loop-var (keyword as loop var is a parse error), then `in`, then expression. Trailing tokens after the iterable expression are a parse error.
- `else if` is a parse error (`else` accepts no operand).
- Empty body range → immediate parse error ("missing expression" for interpolations, "missing statement" for statements).
- Keyword reservation is checked wherever an identifier is accepted in identifier-position (var refs, fn names, loop-var names). Function argument *lists* parse expressions — not the identifier-position check.
- Integer literal parsing: optional leading `-`, decimal, leading zeros allowed, no `+`, must fit signed i64 — out-of-range is a parse error.
- Trailing commas in list literals and function-call argument lists are parse errors.
- `{% end %}` matches the innermost open block via the recursive-descent stack; unclosed openers and orphan `{% end %}` are parse errors.

Expression sub-parser: hand-written recursive descent over `src[body]` bytes directly, with its own cursor — no intermediate sub-token stream. The token-stream cursor of the main parser is unaffected. Error byte offsets are expressed relative to `body.start`.

### Errors and miette integration

```rust
pub enum Error {
    Parse(ParseError),
    Render(RenderError),
    Func(FuncError),
    Io(io::Error),
}
pub type Result<T> = std::result::Result<T, Error>;
```
- Three sub-enums (`ParseError`, `RenderError`, `FuncError`) declare the structured variants per spec § Parse-time errors and § Render-time errors. Top-level `Error` lifts them via `#[from]`.
- Each sub-enum carries a `span: SourceSpan` (byte range) set at construction; no mutation/fix-up pass.
- Function errors attribute to the `FnCall.name` byte range when lifted to `Error::Func` at the call boundary.
- Construction IO (file open, mmap) and render IO (write, flush) both surface as top-level `Error::Io(io::Error)` — no byte span attached.
- API boundaries (`Template::from_file` / `from_bytes` / `render`) wrap errors via `Report::new(err).with_source_code(ByteSource)`.

The `ByteSource` wrapper exposes a custom `miette::SourceCode` impl so miette can render byte-accurate spans without lossy whole-source conversion:
- `ByteSource::Owned(Arc<[u8]>)` — used for parse errors, where the `Template` doesn't exist yet on failure. The source bytes are duplicated into the error (rare path; parse errors are not the hot case).
- `ByteSource::Borrowed(&'a [u8])` — used for render errors, borrowing from the existing `Template`'s `Source` (zero-copy).
- The `read_span` implementation does per-span `String::from_utf8_lossy` on the spanned slice only — bytes outside the span are never decoded.

### Crate dependencies

- Runtime: `memmap2`, `miette`, `thiserror`.
- Dev: `test-case`.
- Drop `serde` and `toml` (legacy `TryFrom<toml::Value>` bridge is gone; the host owns toml→Value conversion).

## Testing Decisions

### What makes a good test

- Tests assert **observable external behavior** — render output bytes, error variants, byte spans — never implementation details (AST shape, scope-stack internal structure, helper function call counts).
- Internal-shape tests colocated with `scan()` and `parse()` are the exception: the AST and `Vec<Token>` shapes *are* the contract for those layers, so testing them is testing external behavior of those layers. These colocated tests live in `src/scanner.rs` and `src/parser.rs` under `#[cfg(test)] mod tests`.
- End-to-end render tests use only the public API (`Template::from_bytes`, `render`) — no internal knowledge.
- Error tests assert on `Error` variant via `matches!` and on byte span via `assert_eq!` against the expected `Range<usize>`. No insta snapshots (templater/crate-level testing deliberately avoids insta, per repo convention for this crate).
- A single `tests/common/mod.rs` provides helpers: a `render` wrapper that takes a source byte slice + variables + registry and returns the rendered `Vec<u8>`; a `MockRegistry` returning `Undefined` for all calls (covers no-function templates); a `TestRegistry` exposing a small set of test functions (`eq`, `not`, `length`, `join`) for tests that exercise function call paths.
- `test-case` is used for matrixed edge-case tests (e.g. escape-rule tables, whitespace-modifier combinations, integer-literal boundary cases).

### Modules under test

- **Scanner (`src/scanner.rs` module tests)** — `Vec<Token>` shape for: six delimiters, escape rule table (all five rows), string-shielding, multi-line tags, `=`/`-` modifier combinations, comment stripping, stray delimiters, `\r` handling, escaped-opening-with-stray-closing interactions.
- **Parser (`src/parser.rs` module tests)** — AST shape for: keyword reservation in identifier positions, empty interpolations/statements, malformed `for` bindings, integer-literal boundaries (overflow, leading zeros, `+7`, `-7`), trailing commas, unclosed/orphan blocks, `else if`, `endif`/`endfor`.
- **Render (`templater/tests/render.rs`)** — end-to-end: canonical output per `Value` type, scoped shadowing, empty iterable, short-circuit if, render-only-on-execution (errors inside `{% if false %}` don't fire), dot-access on each type, list-literal iteration with shadowed loop var.
- **Errors (`templater/tests/error.rs`)** — end-to-end: undefined variable, undefined function, non-Bool condition, non-List iterable, negative index, out-of-bounds index, map key not found, dot-access type errors, wrong arg count, function type mismatch, parse-time errors (each variant from spec § Parse-time errors).

### Prior art in the codebase

- `tests/templater.rs` — root integration tests driving `dotrift::command::templater::run` end-to-end through the CLI subcommand. These cover the host integration (variable source, CLI flags, output streaming) and stay as the host-level seam — out of scope for this spec but referenced as a hat-tip to the highest black-box seam in the repo.
- `templater/legacy/tests/error.rs` — archived prior implementation's error tests; readable for prior-art structure but not copyable.
- `tests/common/` (root) — sibling pattern for shared test helpers; the templater crate's `templater/tests/common/mod.rs` mirrors this layout convention.

## Out of Scope

- Wiring the new templater crate into dotrift's `templater` CLI subcommand, `apply`, `add`, or `unapply` paths. The existing host-level integration tests in `tests/templater.rs` will continue to use the legacy stub until a separate host-wiring effort updates the subcommand and its dependencies.
- The concrete built-in function set dotrift exposes (e.g. `upper`, `lower`, `join`, `eq`, `gt`, `lt`) — that's a host concern.
- Variable ingestion from `dotrift_data.toml` or CLI `--var` flags — host concern.
- Migrating `templater/legacy/` to the new code or removing the legacy directory — separate cleanup effort.
- Replacing miette with another diagnostics framework.
- Adding `Ord`, `Hash`, or `Deserialize` derives to `Value`.
- Optimization beyond the locked decisions (e.g. borrow-fast-path expression evaluation, flat-arena AST with jump indices, arena-backed value storage). These were considered and deferred during the design conversation; revisit only if profiling demands.
- Multi-file patterns like partials, includes, macros, or template inheritance — not in the spec.
- Thread-safe sharing of a single `Template` across threads (it's `Send` if `Source` is; `Sync` similarly; no explicit synchronization is added).

## Further Notes

- The full design conversation ran through 20 numbered question rounds (Q1–Q20), each weighing concrete alternatives and locking one. The "Implementation Decisions" section above is the surviving synthesis; the conversation history is the rationale trail.
- This spec respects the vocabulary in `templater/spec/CONTEXT.md` — *tag*, *delimiter*, *interpolation*, *statement*, *comment*, *plain text*, *escape*, *stray delimiter*, *modifier*, *expression*, *block*, *branch*, *loop variable*, *iterable*, *scope*, *function registry*, *render-time*, *parse-time* — and avoids the listed synonyms.
- No ADR conflict exists: the three existing ADRs (`0001-profile-precedence-by-timestamp.md`, `0002-symlink-of-symlink-deployment.md`, `0003-lexical-path-normalization.md`) cover dotrift's apply/symlink paths, not the templater language.
- The templater's `spec/syntax.md` remains the authoritative behavior contract for any implementation detail this spec doesn't override (e.g. the exact `=` scanner barrier rule, the canonical-form rendering rules for nested aggregate values).
- Crate-level testing rejects insta deliberately (decision I2); the workspace `insta` dependency remains for other crates' use.