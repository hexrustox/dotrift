# Dotrift Agent Guide

Declarative, template-aware dotfile manager written in Rust. Maps files from a
source directory to a target directory via `dotrift.toml`.

## Documentation index

- [Testing conventions](docs/agents/testing.md)
- [Issue tracker](docs/agents/issue-tracker.md)
- [Domain docs](docs/agents/domain.md)

**Authoritative behavior contract:** `spec/*.md` (root) and `<workspace>/spec/*.md` (per workspace member). Consult them before changing CLI, config, DB schema, templater syntax, or pager behavior.

## Workspace overview

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
- Unused deps: `cargo machete` · Supply chain: `cargo deny check` · TOML
  formatting: `taplo format`

There is no required order beyond `fmt` before commit; no CI, no git hooks.
