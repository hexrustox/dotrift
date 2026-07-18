# 01 — Skeleton: plain text roundtrip via `from_bytes`

**What to build:** An end-to-end usable templater crate where `Template::from_bytes(Vec<u8>)` constructs a `Template` that, when rendered, emits the source bytes verbatim to the caller's writer. No tags are recognized yet — the entire byte stream is plain text. This ticket establishes the module layout (`lib`, `ast`, `error`, `eval`, `function`, `parser`, `scanner`, `value`), drops `serde` and `toml` from `templater/Cargo.toml` dependencies, lands the public API surface (`Template`, `Value`, `ValueType`, `FunctionRegistry`, `Error`, `ParseError`, `RenderError`, `FuncError`, `Result`), the `Source::Owned(Vec<u8>)` variant, basic writer buffering/flush behavior on `render`, and the `ByteSource::Owned(Arc<[u8]>)` miette `SourceCode` impl so parse-error spans are renderable (no render errors fire yet). A template with zero tags renders byte-for-byte to the writer, and a template with any tag-shaped bytes still renders verbatim (the scanner is a passthrough in this slice).

**Blocked by:** None — can start immediately.

**Status:** ready-for-agent

- [x] `templater/Cargo.toml` has runtime deps `memmap2`, `miette`, `thiserror`; dev dep `test-case`; `serde` and `toml` removed.
- [x] Modules `lib`, `ast`, `error`, `eval`, `function`, `parser`, `scanner`, `value` exist under `templater/src/` and compile (some may be stubs only as large as this slice needs).
- [x] Public API exports reachable from `templater::`: `Template`, `Value`, `ValueType`, `FunctionRegistry`, `Error`, `ParseError`, `RenderError`, `FuncError`, `Result`.
- [x] `Template::from_bytes(Vec<u8>) -> Result<Template>` constructs a template and stores it via `Source::Owned`.
- [x] `Template::render<W: io::Write>(&self, writer: W, variables: &HashMap<String, Value>, functions: &dyn FunctionRegistry) -> Result<()>` flushes the writer on success and emits the source bytes verbatim.
- [x] `Error` enum has variants `Parse`, `Render`, `Func`, `Io` with `#[from]` conversions from the sub-enums and `io::Error`.
- [x] `ByteSource::Owned(Arc<[u8]>)` implements `miette::SourceCode` via per-span `String::from_utf8_lossy` inside `read_span`.
- [x] End-to-end test in `templater/tests/render.rs`: a source byte slice with no tags renders identically to the input.
- [x] `cargo build -p templater`, `cargo test -p templater`, `cargo fmt`, `cargo clippy -p templater` pass.