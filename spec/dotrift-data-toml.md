# `dotrift_data.toml`

Located in the source directory, next to `dotrift.toml`. Optional. When
present, its variables feed into both `dotrift.toml` template evaluation and
`tmpl`-type source file rendering.

---

## Format

```toml
[variable]
key = "value"

[profile.<name>]
key = "override"
```

## `[variable]`

Key-value pairs forming the base template context. Values can be any valid
TOML type: string, integer, boolean, array, or inline table. These map to
the template value types String, Int, Bool, List, and Map respectively.

## `[profile.<name>]`

Named profiles with additional or overriding variables. Same value types as
`[variable]`. Multiple profiles can be active simultaneously. Activation is
performed by `dotrift profile activate <name>` (see spec/commands/profile.md section "profile");
the set of currently-active profiles is persisted in the `active_profiles`
table (see spec/core.md section "active_profiles Table").

## Profile Resolution

When a template is evaluated, the variable context is built by layering
profiles over the base in activation-timestamp order:

1. `[variable]` is the base context.
2. Active profiles (from the `active_profiles` DB table) overlay on top.
3. Last-activated profile (highest `activated_at`) wins on conflict.
4. Profiles active in DB but missing from `dotrift_data.toml` are silently ignored.

The rationale for timestamp-based precedence (over positional or declaration
order) is recorded in ADR-0001.

## Errors

Parse errors are fatal. A missing file is treated as empty (no variables
defined).
