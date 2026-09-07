# ADR-0017: Per-run template render registry

Rendering a template is a pure function of its bytes and the run's variable
context — the function registry is always empty — so identical inputs within
one `apply` run cannot produce different output; each render is stored once in
a content-addressed directory under the system temp dir, keyed by the xxHash64
of the template bytes. The key deliberately excludes the variable context, so
the registry is valid only for the run that filled it: a real run empties it
on start — a killed run (no signal handling, ADR-0015) could otherwise poison
the next run with output rendered under an old context — and best-effort on
exit. A persistent registry keyed by template bytes plus a context hash was
rejected: cross-run reuse buys little for dotfiles while adding a GC story,
and a stale serve would silently deploy content rendered under an old profile.
Registry-infra failures fall back to direct rendering rather than failing a
run whose target filesystem may be perfectly writable. Template view diffs
are the exception: they diff the registry's bytes only, so a registry
failure fails the view diff like a render failure — view diff has no
fallback, and no second temp-file render path is kept alive solely for
registry-outage runs.
