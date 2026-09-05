# Dotrift Agent Guide

Declarative, template-aware dotfile manager written in Rust. Maps files from a
source directory to a target directory via `dotrift.toml`.

## Agent skills

### Issue tracker

Issues are tracked in GitHub Issues via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical triage-role labels are used as-is. See `docs/agents/triage-labels.md`.

### Domain docs

Multi-context: root `CONTEXT-MAP.md` points at per-context `CONTEXT.md` files, with ADRs in `docs/adr/`. See `docs/agents/domain.md`.

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

## Commands

- Build / check / run: `cargo build`, `cargo check`, `cargo run -- <args>`
- Tests: `cargo test` (all workspace); `cargo test -p templater` or `-p tui`
  for a single crate; `cargo test --test apply` for one integration file.
- Lint/format: `cargo fmt`, `cargo clippy`
