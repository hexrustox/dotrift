# Testing conventions

This document covers testing conventions for the dotrift workspace. See also
[Parametrized tests (test-case)](test-case.md) and
[Property-based tests (proptest)](proptest.md).

## Test locations

Conventions for where tests live, applied per crate:

- **Unit tests** are colocated with the code they exercise in `#[cfg(test)]`
  modules under the crate's `src/`.
- **Integration tests** live in the crate's `tests/` directory and exercise the
  public API. Shared helpers go in `tests/common/`.
- **Property-based tests** (`proptest`) live wherever the invariant fits best —
  colocated unit test or `tests/` — with the auto-generated regression corpus
  in the crate's `proptest-regressions/`.

## Choosing a test style

Decide the *style*, then conditionally read the relevant doc. If the prompt
already names the tool (e.g. "use proptest", "add `#[test_case]`"), jump
straight to its doc below; otherwise choose by volume and shape:

1. **`#[test]`** — a single case, or setup too varied to express as arguments.
   Nothing else to read.
2. **`#[test_case]` / `#[test_matrix]`** — high volume: the same logic exercised
   with many fixed inputs. Read [Parametrized tests (test-case)](test-case.md).
3. **`proptest`** — a cross-cutting invariant over inputs that are hard to
   enumerate but easy to specify. Read [Property-based tests (proptest)](proptest.md).
4. **`insta` snapshots** — asserting large or structural output (rendered
   output, error reports) as golden files. Read [Snapshot testing (insta)](insta.md).
   Composes with the other styles: a `#[test_case]` or proptest test may still
   end in a snapshot assertion.

The style axis composes with the unit-vs-integration axis below: any style works
at either level.

## Unit vs integration tests

Every crate has both colocated unit tests (`#[cfg(test)]`) and integration
tests (`tests/*.rs`). Use this decision order when choosing where a test
belongs:

1. **Unit test** — when the behavior is an internal data transformation with a
   clear boundary (e.g. a scanner, parser, or formatter). Assert exact internal
   shapes that would be awkward through the public API.

2. **Integration test** — when the behavior is user-facing per the relevant
   `spec/*.md`. Assert rendered output bytes, error variants, or error spans
   through the public API.

3. **Both** — only when each level proves something the other cannot. A unit
   test pinning an exact internal shape plus an integration test confirming
   that shape survives the full pipeline is valid. Duplicating the same
   assertion at both levels is not.

## State record construction in tests

Construct [`StateRecord`] through the `record!` macro — `record!(f, target, hash)`
for file records, `record!(s, target, source)` for symlink records — never with
a struct literal. The `f` arm takes no source path: a file record's fingerprint
is its content hash, so tests cannot express a source-dependent file check.
Literals are reserved for `src/state.rs` (schema-violation cases) and
`tests/status.rs`; a bare `StateRecord { ... }` elsewhere is a review flag.
`record!` is exported under `#[cfg(any(test, feature = "testing"))]`, so
integration tests reach it via `dotrift::record!`.
