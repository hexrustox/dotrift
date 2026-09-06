# `profile`

Manages template profiles: selecting, deselecting, listing, and previewing the
resolved variable context. The profile definition format and the layering
algorithm are specified in `spec/dotrift-data-toml.md § Profile resolution`;
the `active_profiles` storage schema is defined in `spec/core.md`; the global
CLI conventions are defined in `spec/commands/global.md`.

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
   `spec/commands/global.md § Output conventions`).

Stale active profiles — names present in `active_profiles` but absent from
the current data file — are not shown; they remain removable via
`deactivate`.

## `activate <name>`

1. Parse `dotrift_data.toml` and validate that `[profile.<name>]` exists. An
   undefined profile name is an error.
2. Acquire the state lock (see `spec/core.md § State lock`).
3. `INSERT OR REPLACE` into `active_profiles` with a fresh `activated_at`
   timestamp, strictly greater than every stored one (see `spec/core.md §
   `spec/core.md § active_profiles Table`). Re-activating an
   already-active profile updates
   its timestamp, moving it to the end of the precedence order.
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
2. Resolve the variable context per
   `spec/dotrift-data-toml.md § Profile resolution`. The context `show`
   prints is exactly the context `dotrift.toml` rendering and deployed
   templates receive.
3. Print the context as a two-column key–value table, keys sorted
   lexicographically. The key column is padded to the longest key length,
   followed by exactly three spaces before the value. Scalars render as
   their plain values; Lists and Maps render in the templater's canonical
   form (see `templater/spec/syntax.md § Interpolation Output`).
4. An empty context prints nothing.

## Exit status

`profile` subcommands exit `0` on success and `1` on error: an undefined
profile name in `activate`, an inactive profile in `deactivate`, an unreadable
or malformed data file, or a state-lock or database failure. `list` and `show`
always succeed: they report state, they do not check it.
