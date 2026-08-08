# Dotrift

A declarative, template-aware dotfile manager that maps files from a source
directory to a target directory via `dotrift.toml`.

## Language

**Source directory**:
Root of the source tree containing `dotrift.toml`, `dotrift_data.toml`,
`.dotriftignore`, and dotfiles. Resolved before `dotrift.toml` is read; the
config cannot set it.
_Avoid_: source-dir (only as CLI/config token), source dir (two words)

**Source path**:
Per-entry path inside the source directory that dotrift deploys from.
_Avoid_: entry source

**Target directory**:
Root of the destination tree. CLI `--target` override wins over
`target-directory` in `dotrift.toml`, which defaults to `$HOME`. Must be
absolute.
_Avoid_: destination, target-dir (only as CLI/config token)

**Target path**:
Per-entry path on disk that dotrift writes to.
_Avoid_: computed target

**Desired deployment**:
The complete set of resolved portal entries `apply` intends to deploy for a
given run: portal resolution, filtered by the ignore file and validated for
collisions, with rules applied. Comparing it against the target directory and
the management state drives `apply`'s decisions.
_Avoid_: plan (generic)

**Portal**:
One entry in the `[portal]` table mapping a source pattern or literal to a
target destination.
_Avoid_: mapping, route

**Stripping prefix**:
Portion of a glob portal key up to but not including the first path component
that contains a wildcard. Removed from matched source paths before appending
the remainder to the portal destination.
_Avoid_: (none)

**Rule**:
One entry in the `[rule]` table keyed by a target-path pattern, carrying
`type` and/or `mode`. Matches resolved target paths only.
_Avoid_: (none — capitalise to distinguish the dotrift concept)

**Deploy type**:
Enum per deployed file: `symlink` | `copy` | `template`. Set by `[rule]`,
defaulting to `symlink` when no rule matches.
_Avoid_: deployment method

**Managed path**:
A target path that dotrift created and whose current fingerprint still matches
the recorded last-applied fingerprint. Only managed paths are replaced
automatically. A previously managed path that was modified since the last
apply no longer matches and is an *obstruction*. A path whose current on-disk
metadata or content cannot be read fails the check and is not a managed path.
_Avoid_: tracked path

**Fingerprint**:
The recorded last-applied state of a target path dotrift created: the link
target for a symlink deploy, or a hash of the deployed bytes for a copy or
template deploy. Directories have no fingerprint. Comparing the current
fingerprint against the record decides whether the path is still managed.
_Avoid_: checksum, hash (when meaning the recorded state)

**State database**:
The single global SQLite file, per user, holding the management state:
one *state record* per managed path plus the active-profile selectors.
Located at `$XDG_STATE_HOME/dotrift/state.sqlite`, falling back to
`$XDG_DATA_HOME/dotrift/state.sqlite`.
_Avoid_: db, database (when meaning the dotrift file)

**State record**:
One row in the `managed_paths` table of the *state database*: the source
path, target path, deployed kind, and fingerprint of a target path dotrift
created. Mirrors the last completed filesystem action.
_Avoid_: entry, database entry

**Management state**:
The collective state stored in the *state database* — all *state records*
plus the active-profile selectors. Comparing it against the target directory
and the desired deployment drives `apply`'s decisions.
_Avoid_: state (generic)

**Managed check**:
The read-only comparison of a target path's current filesystem kind and
fingerprint against its *state record*, deciding whether the path is still a
*managed path*.
_Avoid_: (none)

**Collision**:
Config-time condition where two different portal resolutions produce the same
target path. Halts the program before any filesystem change. Distinct from
*obstruction*.
_Avoid_: (none — distinct from obstruction)

**Structural conflict**:
Config-time condition where two desired target paths place one as an ancestor
of the other (for example `config` and `config/editor`), so they cannot both
exist as deployment targets. Halts the program before any filesystem change,
like a *collision*.
_Avoid_: path conflict, overlap

**Obstruction**:
An existing target path that dotrift does not manage: either untracked, or
previously created by dotrift but modified since the last apply. Blocks
deployment of a resolved entry. Unlike a *collision* — a config-time error —
an obstruction is a runtime condition resolved interactively during `apply`.
_Avoid_: conflict, clash

**Stale path**:
A *managed path* in the target directory that is not part of the *desired
deployment* for the current run: the candidate set for `--clean-up`. Paths
excluded by the ignore file count as stale.
_Avoid_: leftover, orphan

**Relinquish**:
Drop a *state record* for a path dotrift no longer deploys, leaving the file
itself untouched. Happens under `--clean-up` for stale obstructions (modified
files) and for records whose target no longer exists.
_Avoid_: forget, abandon

**Control file**:
One of the three root metadata files in the source directory — `dotrift.toml`,
`dotrift_data.toml`, `.dotriftignore` — that configures dotrift rather than
serving as a deployed dotfile. Implicitly excluded from deployment, but
re-includable via a negated ignore pattern.
_Avoid_: (none)

**Ignore file**:
The optional `.dotriftignore` at the root of the source directory listing
gitignore-style patterns that exclude resolved target paths from deployment.
_Avoid_: dotignore, gitignore (when meaning the dotrift file)

**Ignore pattern**:
One line of the ignore file, in standard gitignore syntax, matched against a
resolved target path. Patterns are evaluated in order; the last match decides.
_Avoid_: ignore rule (distinct from *rule*)

**Base variables**:
The key-value bindings under `[variable]` in `dotrift_data.toml`. The initial
layer of the variable context. Distinct from *profile* and from the resolved
*variable context*.
_Avoid_: defaults, root variables

**Profile**:
A named overlay definition under `[profile.<name>]` in `dotrift_data.toml`,
carrying variable bindings that override the base variables. Distinct from an
*active profile*, which is the persisted selector referencing it.
_Avoid_: layer (a profile is a definition, not the layering itself)

**Active profile**:
A profile selected for inclusion in the variable context, recorded in the
persisted active-profile state as `(name, activated_at)`. May reference a
profile missing from the current data file, in which case it is ignored.
_Avoid_: selected profile, enabled profile

**Variable context**:
The resolved bindings passed to the templater for evaluation: base variables
overlaid by active profiles in `activated_at` order, most recently activated
winning, with lexicographic profile-name tie-breaking.
_Avoid_: scope, environment (overloaded terms in the templater spec)

**State lock**:
Exclusive lock held by any command that reads or mutates the *state database*
— `apply`, `profile activate`, `profile deactivate` — serialising concurrent
invocations. `apply` holds it from reading the control files through
filesystem actions, state updates, and exit; short-lived commands hold it for
the duration of their state mutation. A second invocation that cannot acquire
it fails rather than interleaving operations.
_Avoid_: apply lock (apply is one consumer, not the owner)
