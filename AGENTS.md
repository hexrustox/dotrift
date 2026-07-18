# Dotrift Agent Guide

Declarative, template-aware dotfile manager written in Rust. Maps files from a
source directory to a target directory via `dotrift.toml`. **`spec/*.md` (root) and
`<workspace>/spec/*.md` (per workspace member) are the authoritative behavior
contract** — consult them before changing CLI, config, DB schema, templater
syntax, or pager behavior.

## Workspace layout

Cargo workspace (Cargo.toml):
- `dotrift` (root, bin + lib): `src/main.rs` entrypoint, `src/lib.rs` reexport.
  Subcommands live in `src/command/`; shared helpers in `src/command/util.rs`.
- `tui` (member): interactive prompt + ratatui-based conflict pager, invoked
  on `[d]iff` collisions.
- `templater` (member): the standalone template *engine* (scanner/parser/eval).

### Legacy code

Source files under `legacy/` are archived from a prior implementation. New code may read them for reference but must never copy from them.

## Toolchain

- Rust **1.95**, edition 2024, workspace resolver 3.
- `.cargo/config.toml` forces `clang` + `mold` linker on
  `x86_64-unknown-linux-gnu` — both must be available or builds fail.
- `rusqlite` uses the `bundled` feature; no system SQLite needed.

## Commands

- Build / check / run: `cargo build`, `cargo check`, `cargo run -- <args>`
- Tests: `cargo test` (all workspace); `cargo test -p templater` or `-p tui`
  for a single crate; `cargo test --test apply` for one integration file.
- Lint/format: `cargo fmt`, `cargo clippy`
- Accept changed insta snapshots: `cargo insta review` (snapshots are stored
  relative to the directory of the unit test that produced them — e.g.
  `src/command/snapshots/`, `tui/src/pager/snapshots/`)
- Unused deps: `cargo machete` · Supply chain: `cargo deny check` · TOML
  formatting: `taplo format`

There is no required order beyond `fmt` before commit; no CI, no git hooks.

## Testing conventions

- Integration tests at repo root (`tests/*.rs`, shared `tests/common/`);
  per-crate ones in `tui/tests/` and `templater/tests/`.
- Unit + snapshot tests colocated under `#[cfg(test)]` in `src/`.
- `output!` / `eoutput!` (src/output.rs) capture to a thread-local under
  `#[cfg(test)]` rather than stdout. In tests use
  `output::test_capture::take_all()` or
  `command::util::assert_captured_output(label, temp_path)` (which filters temp
  paths to `@` before snapshotting).
- TUI pager snapshots (tui/src/pager/mod.rs) filter a temp dir placeholder of
  fixed width — use the existing `with_settings!` filters, don't snapshot raw
  paths.

### Parametrized tests

The `test-case` crate (workspace dev-dep, `test-case = "3"`) provides
`#[test_case]` and `#[test_matrix]`. Prefer them over hand-rolled loops or
copy-pasted `#[test]` functions when the same logic is exercised with different
inputs. Authoritative reference for the macro syntax lives in the upstream
wiki — read it before doing anything beyond plain args + label:

- Syntax: <https://github.com/frondeus/test-case/wiki/Syntax>
- Additional attributes & async: <https://github.com/frondeus/test-case/wiki/Additional-Attributes-&-Async>
- Test names: <https://github.com/frondeus/test-case/wiki/Test-Names>

**Import is mandatory.** Every `#[cfg(test)]` module that uses the attribute
must have `use test_case::test_case;` (or `use test_case::test_matrix;`). The
attribute name collides with rustc's `custom_test_frameworks`, so the
un-qualified `#[test_case]` will not resolve without the import.

**Pick the right attribute.**

- `#[test]` — single case, no parameters, or when setup is too varied to
  express as arguments.
- `#[test_case(arg1, arg2 ; "label")]` — one named case per attribute; stack
  several on the same fn. Always supply `; "label"` when the case carries
  meaning: the label becomes the test name in `cargo test` output and is the
  only way to tell cases apart on failure.
- `#[test_matrix([a, b, c], [1, 2])]` — Cartesian product of argument lists
  or `a..b` ranges. Use when every combination is meaningful; use
  `#[test_case]` when each case needs its own label.

**Full shape.** Per the wiki, a single attribute is
`#[test_case(inputs (=> modifiers output_matcher)? (comment)?)]`. The
`=> …` output section lets the test fn return a value and the macro
validate it — use it instead of hand-written `assert_eq!` when the
assertion is a single comparison.

Output matchers available:

- `==` (equality) — `#[test_case(2 => true)]` for `fn is_natural(n: i32) -> bool`.
- `matches Pattern (if guard)?` — `#[test_case(-1 => matches Err(_))]`.
- `panics ("expected msg")?` — `#[test_case(2.0, 0.0 => panics "Division by zero")]`.
- `with |actual| …` — `#[test_case(0.0 => with |i: f64| assert!(i.is_nan()))]`.
- `using path::to::fn` or `using expr_returning_fn` — `#[test_case(2 => using simple_validate)]`.
- Hamcrest-style `is` / `it` matchers: `equal_to`, `less_than`, `greater_than`,
  `less_or_equal_than`, `greater_or_equal_than`, `almost_equal_to` /
  `almost … precision N`, `contains` / `contains_in_order`, `existing_path`,
  `is file` / `is dir` / `is directory`. Combine with `not`, `and`, `or` —
  `and`/`or` cannot be mixed without parentheses.

Modifiers before the matcher: `ignore` or `inconclusive`, optionally with a
reason: `#[test_case(_ => ignore["not implemented"] _)]`.

**Attribute order on the fn is strict.** `#[test_case(...)]` must come
**first**; any other attribute (`#[tokio::test]`, `#[allow(clippy::...)]`,
`#[cfg(...)]`, …) goes **after** it. Reversing the order breaks expansion.
For async tests stack them: `#[test_case(...)]` then `#[tokio::test]`; the
macro generates the test fns and `tokio::test` adapts each one.

**Naming.** With a comment, the comment is the test name (e.g.
`foo::label`). Without a comment, the macro synthesizes a name from the args
and matcher (e.g. `_2_7_expects`), which is unreadable on failure — always
prefer the `; "label"` form. Each attribute expands to exactly one `#[test]`,
so `cargo test` counts grow accordingly; pick labels that survive a CI
search.

## Agent skills

### Issue tracker

Issues live as markdown files under `.scratch/<feature-slug>/` in this repo. See `docs/agents/issue-tracker.md`.

### Domain docs

Multi-context layout — `CONTEXT-MAP.md` at root points at `spec/CONTEXT.md` +
`templater/spec/CONTEXT.md`. ADRs live in `docs/adr/` only (no workspace split).
See `docs/agents/domain.md`.
