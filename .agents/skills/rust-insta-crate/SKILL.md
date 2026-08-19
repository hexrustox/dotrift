---
name: rust-insta-crate
description: Write Rust snapshot tests with the `insta` crate — decide between snapshot assertions and literal `assert_eq!`/`assert!` via a decision tree, pick the right macro (`assert_snapshot!`, `assert_debug_snapshot!`, the serde serializers, `assert_binary_snapshot!`), store snapshots inline or in `.snap` files, redact or filter volatile values, and review pending `.snap`/`.snap.new` files and accept them with `cargo insta accept`. Use when writing, adding, or reviewing Rust tests that pin a value's output, choosing whether a value should be a snapshot or a literal assertion, or handling a snapshot change.
---

# insta

Snapshot testing for Rust: pin a value's output once, and every later run compares against the stored snapshot. The value lives as a string; insta serializes it (or takes your `Debug`/`Display` output) and stores it beside the test in a `.snap` file, and surfaces changes for review.

The decision that matters is **snapshot or literal** — run the tree once per assertion site before writing any macro.

## Snapshot or literal?

Snapshot and `assert_eq!` both pin a value; they differ in where the correctness judgement lives. A literal is read at the test site — every fact visible in the assertion. A snapshot is read in the diff — the stored `.snap` file against the pending `.snap.new`. Decide which placement the value earns.

1. **Is the value a leaf?** A single atomic value — a number, a bool, a short string, an enum variant, an `Option`, a count. The assertion is a direct fact: `assert_eq!(score, 3)`, `assert!(matches!(err, Error::Timeout))`. Pin it literally. A snapshot spends a file and a review round-trip to pin one fact, and buries the meaning away from the test site. Done when every atomic fact is an inline literal.

2. **Is the whole shape the point?** Is the value large and structured — a multi-field struct, serialized JSON/YAML, a rendered or `Debug` string, generated file content — where a behavior change ripples through many fields at once, and correctness is judged on the shape as a whole? Snapshot it: one assertion pins the entire shape, and the `.snap`-to-`.snap.new` diff shows the ripple as a single change instead of a wall of edited `assert_eq!` lines. Done when one macro pins what dozens of literals would, and the diff is where the shape's correctness reads.

3. **Does the value churn with volatility?** Timestamps, uuids, paths, hashes, nondeterministic collection ordering, floats — but a substantial *stable* core is still worth pinning. If the volatile parts can be neutralized — redactions for serde output, filters for string output — snapshot it with the neutering in place (read `references/redactions-and-filters.md`). If the stable remainder is a leaf, go back to branch 1 and assert that leaf literally instead. Done when the snapshot stores only what the test means to pin.

4. **Is only a property at stake, not the content?** The test cares that a predicate holds — `is_ok()`, contains an item, is sorted, a type tag — not what the value is. Assert the property, not the content: `assert!(v.windows(2).all(...))`, `prop_assert!`. Neither a snapshot nor a content literal fits. If the risk is in the input space, write a property test — see `choosing-test-type`. Done when the assertion names the property, not the value.

A value that is large *and* property-checkable answers branch 4: the snapshot pins the whole, the property asserts the invariant — write both.

## Setup

Add insta as a dev-dependency. No features are needed for the core macros; enable a serializer feature only for the format you actually write snapshots in:

```toml
[dev-dependencies]
insta = "1"
```

`assert_json_snapshot!` needs the `json` feature; `toml`, `ron`, `csv`, and `yaml` likewise. Enable `redactions` and `filters` features only when the value has volatile parts (branch 3). Do not try to install `cargo-insta`; use the `cargo insta` subcommands if present, and review snapshot files directly otherwise.

## The macros

Pick by how the value becomes a string:

| You have... | Macro | Requires |
|---|---|---|
| a `&str` / `String` (rendered output, formatted text) | `assert_snapshot!` | — |
| a `Debug` type with no serde impl | `assert_debug_snapshot!` | `Debug` |
| a `serde::Serialize` value meant to be JSON | `assert_json_snapshot!` | `json` feature |
| a `serde::Serialize` value, `toml`/`ron`/`csv`/`yaml` flavor | `assert_toml_snapshot!` / `assert_ron_snapshot!` / `assert_csv_snapshot!` / `assert_yaml_snapshot!` | matching feature |

```rust
#[test]
fn splits_words() {
    let words = split_words("hello from the other side");
    insta::assert_debug_snapshot!(words);
}
```

Redactions are a third argument: `assert_json_snapshot!(value, {".id" => "[uuid]"})`. See `references/redactions-and-filters.md` for the full catalog.

## File and inline snapshots

Snapshots store as a `.snap` file beside the test by default; the *inline* form embeds the stored value as a string literal in the macro call, written as `@"..."`:

```rust
insta::assert_debug_snapshot!(words, @"");
```

After acceptance, the value fills the literal. Inline keeps the expected output at the test site (branch-2 readability at the cost of a long macro call); file snapshots keep the test site clean and the snapshot diffable independently. Inline snapshots update in place only via `cargo insta accept` — with no `cargo-insta` available, prefer file snapshots.

**Naming.** Unnamed snapshots derive from the test function name (leading `test_` stripped); multiple assertions in one function count up (`something`, `something-2`). Name snapshots explicitly when a function holds several or the derived name would not read as the pinned outcome — see `rust-code-style`. File names are `<module>__<name>.snap`.

### Snapshot names in `#[test_case]`

Inside a `#[test_case]` parametrized test, the implicit snapshot name derives from the case with an incrementing counter, so it depends on the order cases are tested and varies between runs. Pin it explicitly from the current test's fully-qualified name — the two-arg named form `assert_snapshot!("name", value)`:

```rust
let test_name = std::thread::current().name().unwrap().replace(":", "_");
assert_snapshot!(test_name, output);
```

`replace(":", "_")` turns the `::` separators in the thread name into `_` — `module_test_fn_case_1` for a case `module::test_fn::case_1` — giving each case a deterministic snapshot name.

## Review workflow

The output is never trusted automatically — that is the point. Never use the interactive `cargo insta review` or `cargo insta test --review`: they need a TTY agents do not have. The agent flow is run, find, read, accept:

1. **Run the tests** — `cargo test`. Failing snapshot assertions are the pending changes; each run writes the new output beside the stored snapshot.
2. **Find the snapshot files** — file snapshots live in a `snapshots/` folder next to the test file (`tests/snapshots/` for an integration test at `tests/foo.rs`). The stored snapshot is `<module>__<name>.snap`; a pending change is written alongside as `<module>__<name>.snap.new` (the `INSTA_UPDATE` default, `auto`, writes `.snap.new` outside CI).
3. **Read and review the diff** — read each `.snap` (the stored expectation) against its `.snap.new` (the new output). The judgement lives here, the same review the interactive tool would give: is the change intended behavior, or a regression?
4. **Accept or reject** — if every pending snapshot is correct, run `cargo insta accept` (non-interactive; accepts all pending, moving each `.snap.new` over its `.snap`). If any are wrong, run `cargo insta reject` to drop all pending, fix the code, and re-run from step 1.

## Pointers

- Read `references/redactions-and-filters.md` when the value has volatile parts (decision-tree branch 3): selectors, static/dynamic/sorted/rounded redactions, and regex filters.
- Decide *whether* the test exists and its type with `choosing-test-type` (branch 4 of this tree points here for property tests).
- Name tests and snapshots per `rust-code-style`.
- Docs: https://insta.rs/docs/ · API: https://docs.rs/insta
