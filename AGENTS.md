# Dotrift Agent Guide

Declarative, template-aware dotfile manager written in Rust. Maps files from a
source directory to a target directory via `dotrift.toml`.

## Documentation index

- [Issue tracker](docs/agents/issue-tracker.md)
- [Domain docs](docs/agents/domain.md)

**Authoritative behavior contract:** `spec/*.md` (root) and `<workspace>/spec/*.md` (per workspace member). Consult them before changing CLI, config, DB schema, templater syntax, or pager behavior.

## Workspace overview

Cargo workspace (Cargo.toml):
- `dotrift` (root, bin): `src/main.rs` entrypoint.
- `tui` (member): interactive prompt library.
- `templater` (member): the standalone template engine.
- `demo` (member): `prompt` bin exercising the interactive apply prompt.

### Legacy code

Source files under `legacy/` are archived from a prior implementation. New code
may read them for reference but must never copy from them. The same applies to
specs archived under `legacy/` — reference only, never authoritative; new specs
live in `spec/`.

## Directory conventions

- Root crate `dotrift`: library modules live flat in `src/*.rs`; subcommand
  handlers in `src/commands/`; `src/main.rs` stays a thin entrypoint.
- Integration tests sit in root `tests/`, one file per command/behavior;
  shared helpers in `tests/common/mod.rs`, insta snapshots in
  `tests/snapshots/`.
- Workspace members are self-contained in their directory (`tui/`,
  `templater/`, `demo/`): same `src/` + `tests/` split, plus a per-crate
  `spec/` (authoritative) and `legacy/` archive. Benches only exist in
  `templater/benches/`.
- Extra binaries go in `<crate>/src/bin/` (e.g., `demo/src/bin/prompt.rs`).

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
- Unused deps: `cargo machete` · Supply chain: `cargo deny check` · TOML
  formatting: `taplo format`

There is no required order beyond `fmt` before commit; no CI, no git hooks.
