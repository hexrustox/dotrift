# Testing conventions

## State record construction in tests

Construct [`StateRecord`] through the `record!` macro — `record!(f, target, hash)`
for file records, `record!(s, target, source)` for symlink records — never with
a struct literal. The `f` arm takes no source path: a file record's fingerprint
is its content hash, so tests cannot express a source-dependent file check.
Literals are reserved for `src/state.rs` (schema-violation cases) and
`tests/status.rs`; a bare `StateRecord { ... }` elsewhere is a review flag.
`record!` is exported under `#[cfg(any(test, feature = "testing"))]`, so
integration tests reach it via `dotrift::record!`.