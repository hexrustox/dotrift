# `dotrift_data.toml`

Defines the template variable context. The file is discovered at the root of
the already-resolved source directory, alongside `dotrift.toml` and
`.dotriftignore`. It is optional: a missing file contributes an empty base
variable set and no profiles.

Unlike `dotrift.toml`, this file is never evaluated as a template. It is the
source of the variable context, so templating it would be circular; it is
parsed as plain TOML. No environment variables are injected implicitly.

```toml
[variable]
key = "value"

[profile.<name>]
key = "override"
```

## `[variable]`

Key-value pairs forming the *base variables*.

* **Keys:** any non-empty TOML key. Keys are preserved verbatim, even when
  the templater cannot reference them with normal variable syntax; they remain
  accessible as map members where applicable.
* **Values:** one of the supported value types (see [Value types](#value-types)).
* **Optional:** a missing or empty table is a valid no-op.

## `[profile.<name>]`

Named *profiles*: overlay variable bindings layered over the base variables
when the profile is active.

* **Name:** any non-empty TOML table key. Profile names are exact selectors
  matched against the persisted active-profile state; quote the name when TOML
  syntax requires it (for example `[profile."work/linux"]`).
* **Keys and values:** same rules as `[variable]`.
* **Multiple profiles** can be active simultaneously. Activation is performed
  by `dotrift profile activate <name>` (see spec/commands/profile.md); the set
  of currently-active profiles is persisted in the `active_profiles` table (see
  spec/core.md section "active_profiles Table"). This spec owns the data file
  and the layering rules only; the storage schema and the activation command
  are defined in their own specs.
* **Optional:** a missing or empty table is a valid no-op.

## Value types

Supported values are the templater's value types:

* **String** — any TOML string.
* **Int** — any TOML integer.
* **Bool** — any TOML boolean.
* **List** — a TOML array of supported values. Lists may be heterogeneous and
  may nest.
* **Map** — a TOML table or inline table whose values are supported types.
  Keys are strings and may be arbitrary; values may nest.

Floats, dates, datetimes, and times are unsupported: the templater has no
representation for them, so they are configuration errors rather than being
coerced.

## Profile resolution

When a template is evaluated (whether `dotrift.toml` or a deployed template),
the variable context is built by layering profiles over the base variables:

1. The base variables from `[variable]` form the initial context.
2. Each active profile overlays its bindings onto the context.
3. Active profiles are applied in ascending `activated_at` order, so the
   most recently activated profile wins on conflict.
4. Equal timestamps are ordered lexicographically by profile name; the
   lexicographically later name wins the tie.
5. Profiles named in the active-profile state but absent from the current
   `dotrift_data.toml` are silently ignored; stale activation state never makes
   an unrelated deployment unusable.

### Overlay semantics

A profile binding replaces the whole variable. There is no recursive merge:
if a profile defines `settings`, the base `settings` value — whether a map, a
list, or a scalar — is replaced entirely.

## Validation

Errors halt execution before any filesystem change.

* **Unreadable file:** an I/O error halts execution. Missing is the only
  absence treated as "no data defined".
* **Malformed TOML:** a parse error halts execution.
* **Unsupported value:** a float, date, datetime, or time value is a
  configuration error.
* **Unknown structure:** any root table or key other than `[variable]` and
  `[profile.<name>]` is rejected. Typos such as `[variables]` or `[profiles]`
  fail rather than silently changing the variable context.
