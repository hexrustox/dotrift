# Snapshot testing (insta)

`insta` (workspace dev-dep, `insta = "1.47.2"`, feature `filters`) records
assertions as golden `.snap` files. Use it when the thing you're asserting is
large or structural — rendered output, error reports, anything noisy or brittle
to inline as `assert_eq!`. It composes with every style in
[testing.md](testing.md): a `#[test_case]` or proptest test may still end in a
snapshot assertion.

Resources:

- Book: <https://insta.rs/docs/>
- Crate docs: <https://docs.rs/insta/latest/insta/>

## Workflow

1. Write `insta::assert_snapshot!(…)` (optionally with a name) in a test.
2. Run the test — it fails, writing a pending `.snap.new`.
3. Review with `cargo insta review` (accept/reject interactively), or `cargo
   insta accept` for all; `cargo insta test` fails on pending snapshots (CI).
4. Snapshots live next to the test that produced them: integration tests in
   `tests/snapshots/`, unit tests beside their `#[cfg(test)]` module.

## Naming

Always pass an explicit name to `assert_snapshot!` when combined with
`#[test_case]`: without one, insta derives the snapshot name from the generated
test fn, and that name changes whenever the case inputs change.

Use the current test's name as a stable, readable choice:

```rust
#[test_case(1; "one")]
#[test_case(2; "two")]
fn render_number(n: u32) {
    insta::assert_snapshot!(std::thread::current().name().unwrap(), render(n));
}
```

`std::thread::current().name().unwrap()` is the generated test fn name, which
ends in the case label, so each case lands in its own snapshot — `*__one.snap`
and `*__two.snap`, where `*` is the module + test fn prefix and the suffix is
the label.

## Snapshot hygiene

Never snapshot unstable output: temp or absolute paths, nondeterministic data.
Filter placeholders before snapshotting — `with_settings!` (feature `filters`)
provides the redaction machinery — and reuse the crate's existing filters
rather than inventing new ones.
