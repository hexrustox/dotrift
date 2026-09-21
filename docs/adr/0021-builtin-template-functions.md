# ADR-0021: Builtin template functions

The templater's function registry — which ADR-0017 assumed always empty — is
filled with a fixed set of dotrift builtins, identical for `dotrift.toml`
rendering and deployed templates because both consume the same render
pipeline. The set lives host-side in dotrift rather than in the
templater crate, whose spec deliberately keeps the registry an
unopinionated, host-provided table. Keeping the registry empty was
rejected: dotfiles routinely need environment- and profile-dependent logic
that variables alone express awkwardly. The environment-reading builtins make rendering a
function of the process environment as well as the template bytes and the
variable context, which is safe for ADR-0017's per-run registry because the
environment is constant for the process's lifetime — identical inputs still
cannot produce different output within one run — and each run empties the
registry on start, disposing of any previous run's renders under a stale
environment. Injecting the environment into the variable context instead
was rejected: it would make every variable lookup environment-dependent and
widen the impurity beyond the environment-reading builtins.
