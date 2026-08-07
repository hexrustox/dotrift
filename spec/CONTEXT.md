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

**Collision**:
Config-time condition where two different portal resolutions produce the same
target path. Halts the program before any filesystem change. Distinct from
*obstruction*.
_Avoid_: (none — distinct from obstruction)

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
