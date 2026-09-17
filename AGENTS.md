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
  and doesn't meet current standards. `TEST-REWRITE-PLAN.md` tracks the
  port; done with a legacy lookup when the spec answer supersedes it.
- `spec/` is the behavioral contract. Use `spec/CONTEXT.md` for domain terms
  and `spec/commands/<command>.md` when changing that command; `templater/`
  and `tui/` carry their own specs in their own dirs.

## Decisions

- Use `docs/adr/` before changing apply, preflight, or template semantics;
  ADRs explain non-obvious rules there (render-before-parse, apply-never-prunes,
  preflight best-effort). Done when the change matches the relevant ADR or
  an update to it is included.
