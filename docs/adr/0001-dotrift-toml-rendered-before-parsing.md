# ADR-0001: `dotrift.toml` is template-rendered before parsing

`dotrift.toml` is evaluated as a template using the same resolved variable
context as deployed templates (base variables from `dotrift_data.toml` plus
active profiles) before it is parsed as TOML. This was a deliberate reversal
of an earlier decision to keep the configuration static. Because the effective
portals and rules can vary with the active profile context, all validation —
collisions, path-safety rules, and rule contradictions — runs on the rendered
configuration. The alternative, keeping the configuration as static TOML,
would have locked deployment topology to a single machine's profile state and
forced users to maintain per-machine config files.
