# ADR-0006: Deployed templates render immediately before the write step

Deployed template entries are rendered immediately before the write step of
their own deploy action, not during preflight. A render error therefore fails
that action mid-run, after earlier entries may already have changed the
filesystem — consistent with the no-rollback model of ADR-0005. Because state
mirrors completed filesystem actions (ADR-0007), a render failure after an
obstruction was already removed leaves the target absent with its state
removed. The alternative, rendering
every template in preflight so any template error halts before any change, was
rejected for the cost it would impose: every template evaluated eagerly on
every run, including in `--dry-run`, where rendering is deliberately skipped.
`dotrift.toml` is unaffected; per ADR-0001 it is still rendered before parsing.
