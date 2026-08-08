# ADR-0009: Preflight rejects structural target conflicts

Two desired target paths may not place one as an ancestor of the other. The
*collision* check catches identical target paths, but `config` and
`config/editor` are distinct paths that cannot both exist as deployment
targets: whichever deployed first would occupy a path the other needs, making
the outcome order-dependent — and deployment order is a function of target-path
sorting, not of user intent. Preflight therefore rejects ancestor/descendant
target pairs as configuration errors before any filesystem change. The
alternative, allowing them and reconciling at runtime, was rejected because
runtime resolution would destroy one desired entry to satisfy another, with no
safe fallback.
