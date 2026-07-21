# Testing conventions

This document covers testing conventions for the dotrift workspace.

## Test locations

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

## Parametrized tests

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

## Crate-specific guidance

- **templater** — see [`templater/AGENTS.md`](../../templater/AGENTS.md) for unit vs integration test decision framework.
