# Property-based tests (proptest)

`proptest` is a property-testing framework (QuickCheck family, inspired by
Hypothesis). Instead of hand-picking inputs, you state a *property* that must
hold for all inputs and let proptest generate them; on failure it *shrinks*
the input to a minimal repro. It complements — never replaces — the
hand-picked unit tests.

Resources:

- Book: <https://proptest-rs.github.io/proptest/intro.html>
- API docs: <https://docs.rs/proptest/latest/proptest/index.html>

## Mental model

Proptest is built from three pieces (book tutorial, §1.2):

- A **`Strategy`** describes both how to generate a value and how to shrink it
  to simpler forms. Ranges are strategies (`0..100i32`); string literals are
  strategies for strings matching them (`"[0-9]{4}-[0-9]{2}-[0-9]{2}"`); most
  types have built-ins via `any::<T>()`.
- A **`ValueTree`** is one generated value plus `simplify()` / `complicate()`,
  used to walk toward the minimal failing input.
- A **`TestRunner`** drives everything: `runner.run(&strategy, |v| …)` runs
  many cases, catches panics, and shrinks any failure to the minimal case
  (returned as `TestError::Fail`).

The **`proptest!` macro** is sugar over a `TestRunner`. Each `name in strategy`
argument becomes an input; multiple inputs are grouped into a compound (tuple)
strategy:

```rust
proptest! {
    #[test]
    fn int_literal_renders_identity(n in any::<i64>()) {
        // body runs once per generated value of `n`
    }
}
```

## Working with strategies

- Import `proptest::prelude::*` for the common macros and combinators.
- Strategies compose: tuples, arrays, and `proptest::collection::vec` /
  `vec_map`; `prop_oneof!` picks between alternatives; `.prop_map()`,
  `.prop_flat_map()`, and `prop_compose!` build derived strategies.
- Prefer `prop_assert!` / `prop_assert_eq!` — same as `assert!`/`assert_eq!`,
  but the failure report names the generated inputs, e.g.
  `minimal failing input: y = 0, m = 10, d = 1`.
- Assertion failures inside the body are caught by the runner, which reports
  the *minimal* input, not the first random one.

## Failure persistence

On failure, proptest persists the seed in `proptest-regressions/` (keyed by
the source file of the test) and replays it on later runs, so the failure
can't spuriously pass. **Commit these files.** By default all tests in a crate
share one persistence file — split with the `failure_persistence` `Config`
option if a large test count slows runs.

## When not to reach for proptest

- **Known edge cases.** Property tests sample the space; a single-value case
  like `i64::MIN` is virtually never hit. Keep hand-written cases for values
  you know matter (book §1.7).
- **Readable spec examples.** Prefer explicit unit/integration examples for
  spec features; proptest is for cross-cutting invariants that are hard to
  enumerate but easy to specify.
