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
`target-directory` in `dotrift.toml`, which defaults to the user's home
directory. Must be absolute.
_Avoid_: destination, target-dir (only as CLI/config token)

**Target path**:
Per-entry path on disk that dotrift writes to.
_Avoid_: computed target

**Desired deployment**:
The complete set of resolved portal entries `apply` intends to deploy for a
given run: *portal resolution*, filtered by the ignore file and validated for
collisions and structural conflicts, with rules applied. Comparing it against
the target directory and the management state drives `apply`'s decisions.
_Avoid_: plan (generic)

**Portal**:
One entry in the `[portal]` table mapping a source pattern or literal to a
target destination.
_Avoid_: mapping, route

**Portal resolution**:
The activity that turns `[portal]` keys into source-path/target-path pairs by
walking the source directory. Source symlinks are transparent to it —
traversal follows them — and a file reached through one deploys under its
logical source path.
_Avoid_: discovery, scanning

**Stripping prefix**:
Portion of a glob portal key up to but not including the first path component
that contains a wildcard character. Removed from matched source paths before
appending the remainder to the portal destination.
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
The recorded last-applied state of a target path dotrift created: the source
path for a symlink deploy, or a hash of the deployed bytes for a copy or
template deploy. Directories have no fingerprint. Comparing the current
fingerprint against the record decides whether the path is still managed.
_Avoid_: checksum, hash (when meaning the recorded state)

**State database**:
The single global SQLite file, per user, holding the management state:
one *state record* per managed path plus the active-profile selectors.
Located at `$XDG_STATE_HOME/dotrift/state.sqlite`, falling back to
`$HOME/.local/state/dotrift/state.sqlite`, then to
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

**Identical obstruction**:
An *obstruction* whose on-disk content equals what would be deployed for the
entry: for a file deploy, a path that resolves to a regular file whose
content fingerprint equals the fingerprint of the bytes that would be
deployed; for a symlink deploy, a symlink whose link target equals the source
path. File mode is not part of the comparison.
_Avoid_: identical file, matching target

**Stale path**:
A *managed path* in the target directory that is not part of the *desired
deployment* for the current run: the candidate set for `--clean-up`. Paths
excluded by the ignore file count as stale.
_Avoid_: leftover, orphan

**Relinquish**:
Drop a *state record* for a path dotrift no longer deploys, leaving the file
itself untouched. Happens under `--clean-up` for stale obstructions (modified
files) and for records whose target no longer exists. Without `--clean-up`,
such records persist until a future deploy reuses the path.
_Avoid_: forget, abandon

**Control file**:
One of the three root metadata files in the source directory — `dotrift.toml`,
`dotrift_data.toml`, `.dotriftignore` — that configures dotrift rather than
serving as a deployed dotfile. Implicitly excluded from deployment when mapped
to the target-directory root (the implicit ignore patterns are root-anchored
target paths); a portal may still deploy one to a nested target path.
Re-includable at the root via a negated ignore pattern.
_Avoid_: (none)

**Global config**:
The optional per-user TOML file that configures dotrift across all source
directories, at `$XDG_CONFIG_HOME/dotrift/config.toml` (falling back to
`$HOME/.config/dotrift/config.toml`). Distinct from the *control files*,
which configure a single source directory.
_Avoid_: user config, dotriftrc, config file (generic)

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
winning, with lexicographic profile-name tie-breaking. Resolved once per run;
`dotrift.toml` rendering, deployed templates, and `profile show` all consume
the same context.
_Avoid_: scope, environment (overloaded terms in the templater spec)

**Template hash**:
The digest of a template's source bytes, computed before render and following
symlinks. Keys the *template render registry* and the run's in-memory memo of
rendered-output digests. Distinct from *fingerprint* (the hash of deployed
bytes).
_Avoid_: pre-render hash, source hash

**Template render registry**:
The per-run store of rendered template output, keyed by *template hash*,
consulted by template deploys and template diffs before rendering. Not part
of the *state database*.
_Avoid_: render cache, cache (generic)

**State lock**:
Exclusive lock serialising mutations of the *state database* — held by `apply`
for its entire run and by `profile activate`/`profile deactivate` for the
duration of their state mutation. Read-only commands (`status`,
`profile list`, `profile show`) read the database without the lock. A second
mutation-holding invocation that cannot acquire it fails rather than
interleaving operations.
_Avoid_: apply lock (apply is one consumer, not the owner)
