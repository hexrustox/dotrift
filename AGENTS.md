## Commands

- Run the affected tests, fix failures caused by the requested change, and
  rerun them without asking at each step. Done when they pass. The suite uses
  disposable fixtures and touches nothing outside `target/`; fixture rules
  live in `docs/agents/test-data.md`.
- Snapshot tests use `insta`; accept changed snapshots with `cargo insta review`.

## Scope

- Operate inside this repo; `target/` is generated and never edited.
- Use `legacy/` (in this root, `templater/legacy/`, and `tui/legacy/`) as
  read-only reference when behavior questions aren't answered by `spec/`;
  never copy code from it into current sources — it predates the rewrite
  and doesn't meet current standards.
- `spec/` is the behavioral contract. Use `spec/CONTEXT.md` for domain terms
  and `spec/commands/<command>.md` when changing that command; `templater/`
  and `tui/` carry their own specs in their own dirs.

## Agent skills

### Issue tracker

Issues live as GitHub issues, managed via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical triage-role labels are used verbatim. See `docs/agents/triage-labels.md`.

### Domain docs

Multi-context: `CONTEXT-MAP.md` points at per-context `CONTEXT.md` files; ADRs in `docs/adr/`. See `docs/agents/domain.md`.
