# 08 — Non-UTF-8 source tolerance and final acceptance sweep

**What to build:** End-to-end verification that the engine accepts non-UTF-8 bytes in plain text and inside string literals — raw bytes flow through the scanner, parser, and writer without preprocessing. Close any spec edge cases uncovered by earlier tickets: nested `for` shadowing across multiple levels, `\r` in `=` scan paths, deep template nesting (within Rust's default stack), interactions between escape rules and multi-line string literals. Run the full quality suite (`cargo fmt`, `cargo clippy`, `cargo test -p templater`, `cargo machete`, `cargo deny check`) and resolve any findings. The crate is left in a host-ready state — no remaining TODOs against `templater/spec/syntax.md`, no dead code, no unused dependencies, all spec parse-time and render-time error variants exercised by at least one test.

**Blocked by:** 07

**Status:** ready-for-agent

- [ ] End-to-end test in `templater/tests/render.rs` covers a template with non-UTF-8 bytes in plain text emitted verbatim through `render`.
- [ ] End-to-end test in `templater/tests/render.rs` covers non-UTF-8 bytes inside a string literal interpolated via `{{ "..." }}`.
- [ ] End-to-end tests cover nested `for` shadowing (inner loop's loop var restores outer loop's loop var after `{% end %}`).
- [ ] End-to-end tests cover `\r` handling in `=` scan paths (plain text, not a line terminator).
- [ ] Every parse-time error variant in spec § Parse-time errors has at least one assertion in `templater/tests/error.rs` (or colocated parser tests) exercising it via `matches!` + byte span equality.
- [ ] Every render-time error variant in spec § Render-time errors has at least one assertion in `templater/tests/error.rs` exercising it via `matches!` + byte span equality.
- [ ] `cargo fmt`, `cargo clippy -p templater -- -D warnings`, `cargo test -p templater`, `cargo machete`, `cargo deny check` all pass clean.
- [ ] No remaining TODOs against `templater/spec/syntax.md` in the templater crate; no unused dependencies in `templater/Cargo.toml`.