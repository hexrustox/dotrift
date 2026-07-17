# Dotrift Agent Guide

Declarative, template-aware dotfile manager written in Rust. Maps files from a
source directory to a target directory via `dotrift.toml`. **`spec/*.md` is the
authoritative behavior contract** — consult it before changing CLI, config, DB
schema, templater syntax, or pager behavior.

## Workspace layout

Cargo workspace (Cargo.toml):
- `dotrift` (root, bin + lib): `src/main.rs` entrypoint, `src/lib.rs` reexport.
  Subcommands live in `src/command/`; shared helpers in `src/command/util.rs`.
- `tui` (member): interactive prompt + ratatui-based conflict pager, invoked
  on `[d]iff` collisions.
- `templater` (member): the standalone template *engine* (scanner/parser/eval).

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
- Accept changed insta snapshots: `cargo insta review` (snapshots are stored
  relative to the directory of the unit test that produced them — e.g.
  `src/command/snapshots/`, `tui/src/pager/snapshots/`)
- Unused deps: `cargo machete` · Supply chain: `cargo deny check` · TOML
  formatting: `taplo format`

There is no required order beyond `fmt` before commit; no CI, no git hooks.

## Testing conventions

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

## Agent skills

### Issue tracker

Issues live as markdown files under `.scratch/<feature-slug>/` in this repo. See `docs/agents/issue-tracker.md`.

### Domain docs

Single-context layout — one root `CONTEXT.md` + `docs/adr/`. See `docs/agents/domain.md`.
