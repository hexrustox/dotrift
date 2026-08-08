# ADR-0008: `apply` never removes stale entries; cleanup is deferred to `--clean-up`

`apply` adds and updates managed paths but never removes a managed path that is
no longer produced by the desired deployment. Removal semantics — which stale
paths to delete, and under what safety conditions — are deferred to a separate
`--clean-up` flag design rather than folded into `apply`. The obvious
alternative, having `apply` converge the target directory by deleting anything
it no longer manages, was set aside during the initial design because deletion
is the least reversible operation dotrift performs and deserves its own
decision process.
