# `profile`

Manages template profiles: selecting, deselecting, listing, and previewing the
resolved variable context. The profile definition format and the layering
algorithm are specified in `dotrift_data.toml` (Profile Resolution); the
`active_profiles` storage schema is defined in `core.md`; the global CLI
conventions are defined in `global.md`.

**Usage:** `dotrift profile <SUBCOMMAND>`

**Subcommands:**

* `list` — show all profiles defined in `dotrift_data.toml`, marking active
  ones.
* `activate <name>` — activate a profile.
* `deactivate <name>` — deactivate a profile.
* `show` — print the resolved variable context.

## `list`

1. Parse `dotrift_data.toml`; a missing or empty file is a valid empty
   state with no profiles.
2. Query the `active_profiles` table.
3. Print each defined profile name, sorted lexicographically. Profiles
   present in the active set are annotated `(active)`, colored green (see
   `global.md` § Output conventions).

Stale active profiles — names present in `active_profiles` but absent from
the current data file — are not shown. A defined profile that is active but
whose definition has since been removed is likewise invisible to `list`; it
remains removable via `deactivate`.

## `activate <name>`

1. Parse `dotrift_data.toml` and validate that `[profile.<name>]` exists. An
   undefined profile name is an error.
2. Acquire the state lock.
3. `INSERT OR REPLACE` into `active_profiles` with a fresh `activated_at`
   timestamp. Re-activating an already-active profile updates its timestamp,
   moving it to the end of the precedence order.
4. Release the state lock.
5. Print `profile `<name>` activated`.

## `deactivate <name>`

Operates on the `active_profiles` table alone; `dotrift_data.toml` is not
read, so a stale profile whose definition has been deleted can still be
deactivated.

1. Acquire the state lock.
2. Delete the row keyed by `<name>` from `active_profiles`. If no such row
   exists, error: the profile is not active.
3. Release the state lock.
4. Print `profile `<name>` deactivated`.

## `show`

1. Parse `dotrift_data.toml` (missing file contributes an empty base set and
   no profiles) and query `active_profiles`.
2. Resolve the variable context per `dotrift_data.toml` Profile Resolution:
   base variables overlaid by active profiles in ascending `activated_at`
   order, tie-broken lexicographically by name.
3. Print the context as a two-column key–value table, keys sorted
   lexicographically. Scalars render as their plain values; Lists and Maps
   render in the templater's canonical form (see `templater/spec/syntax.md`
   § Interpolation Output).
4. An empty context prints nothing.
