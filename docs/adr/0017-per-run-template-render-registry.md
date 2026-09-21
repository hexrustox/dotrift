# ADR-0017: Per-run template render registry with builtin template functions

Rendering a template is a pure function of its bytes, the run's variable
context, and the process environment — reached only through the builtin
template functions (see `spec/builtin-functions.md`) — so identical inputs
within one `apply` run cannot produce different output; each render is stored
once in a content-addressed directory under the system temp dir, keyed by the
xxHash64 of the template bytes. The key deliberately excludes the variable
context, so the registry is valid only for the run that filled it: a real run
empties it on start — a killed run (no signal handling, ADR-0015) could
otherwise poison the next run with output rendered under an old context — and
best-effort on exit. A persistent registry keyed by template bytes plus a
context hash was rejected: cross-run reuse buys little for dotfiles while
adding a GC story, and a stale serve would silently deploy content rendered
under an old profile. Registry-infra failures fall back to direct rendering
rather than failing a run whose target filesystem may be perfectly writable.
Template view diffs are the exception: they diff the registry's bytes only,
so a registry failure fails the view diff like a render failure — view diff
has no fallback, and no second temp-file render path is kept alive solely for
registry-outage runs.

The templater's host-provided function registry is filled with a fixed set of
dotrift builtin functions rather than left empty. The set lives host-side in
dotrift (see `spec/builtin-functions.md`) rather than in the templater crate,
whose spec deliberately keeps the registry an unopinionated, host-provided
table. Keeping the registry empty was rejected: dotfiles routinely need
environment- and profile-dependent logic that variables alone express
awkwardly. The environment-reading builtins make rendering a function of the
process environment as well as the template bytes and the variable context,
which is safe for the per-run registry because the environment is constant
for the process's lifetime — identical inputs still cannot produce different
output within one run — and each run empties the registry on start, disposing
of any previous run's renders under a stale environment. Injecting the
environment into the variable context instead was rejected: it would make
every variable lookup environment-dependent and widen the impurity beyond the
environment-reading builtins.
