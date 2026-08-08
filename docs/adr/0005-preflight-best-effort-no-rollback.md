# ADR-0005: `apply` validates in preflight, then executes best-effort without rollback

`apply` performs all configuration, resolution, collision, structural-conflict,
and source validation before the first filesystem change, so config errors
never leave a half-applied state. Once execution begins, a failure stops the
run but does not roll back completed changes, does not retry, and does not
re-plan: already completed filesystem actions stand, and the failing entry's
state reflects exactly the actions that completed (ADR-0007). The alternative
— a transaction with rollback — was rejected as more dangerous than the failure
it mitigates: undoing a `replace` that deleted an unmanaged directory can
destroy data the user accepted removal of, and re-running the same operations
is rarely safe.
