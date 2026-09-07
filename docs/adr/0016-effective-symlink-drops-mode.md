# ADR-0016: Effective symlink mode is dropped with a warning

Rules set `type` and `mode` property-by-property, so a mode from one Rule can
meet `type = "symlink"` from another; that effective combination used to halt
the whole deployment as a configuration error. An explicit `mode` beside
`type = "symlink"` within one Rule remains an error — a single-spot
contradiction is most likely a mistake and is cheapest to catch at parse
time. The cross-Rule combination is now a warned no-op: mode is meaningless
for symlinks, so it is dropped and a warning names the affected target rather
than rejecting an otherwise valid configuration.
