# Dotrift

A declarative, template-aware dotfile manager that maps files from a source directory to a target directory via `dotrift.toml`.

## Language

**Source directory**:
Root of the source tree containing `dotrift.toml`, `dotrift_data.toml`, and dotfiles. Defaults to `~/.local/share/dotrift`; overridable via `--source`.
_Avoid_: source-dir (only as CLI/config token), source dir (two words)

**Source path**:
Per-entry absolute path inside the source directory that dotrift deploys from. Stored as `source_path` in the database.
_Avoid_: entry source

**Target directory**:
Root of the destination tree. Resolved by CLI `-t` flag > `target-directory` in `dotrift.toml` > `$HOME`. Must be absolute and never inside the source directory.
_Avoid_: destination (different concept in `add`), target-dir (only as CLI/config token)

**Target path**:
Per-entry absolute path on disk that dotrift writes to. Primary key of the `managed_files` database table.
_Avoid_: computed target (only within the `add` subcommand)

**Portal**:
One entry in the `[portal]` table mapping a source glob or literal to a target destination.
_Avoid_: mapping, route

**Rule**:
One entry in the `[rule]` table keyed by a target-path glob, carrying `type` and/or `mode` properties.
_Avoid_: (none — capitalise to distinguish the dotrift concept from "rule" in the general sense)

**Deploy type**:
Enum on each managed file: `symlink` | `copy` | `tmpl`. Set by `[rule]` and defaulting to `symlink` when no rule matches.
_Avoid_: deployment method

**Tracked**:
Present in the `managed_files` database table, regardless of on-disk state. See also: *managed*.
_Avoid_: (none — distinct from managed)

**Managed**:
Tracked *and* on-disk state matches the database record per the managed check (`spec/core.md#managed-check`): symlink target matches, or content hash matches on-disk (with mtime fast-path). See also: *tracked*.
_Avoid_: in-sync

**Identical**:
Target on disk already equals what dotrift *would write on this run*, irrespective of database state. `symlink`: link target equals the source path. `copy`: hash of source content equals hash of target content. `tmpl` is **never** identical (rendered output depends on active profiles).
_Avoid_: already-applied

**Unmanaged**:
Not managed: either not in the database, or in the database with on-disk state differing from the record.
_Avoid_: drifted (covers only the tracked case)

**Collision**:
Config-time condition where two different source paths resolve to the same target path during portal resolution. Halts the program. Also recorded as `# CONFLICT` comments in `dotrift.toml` by the `add` subcommand. See also: *obstruction*.
_Avoid_: (none — distinct from obstruction)

**Obstruction**:
Runtime condition where something already exists on disk at a target path dotrift wishes to write to (file, directory, dangling symlink, etc.). Triggers the Skip / Overwrite / Diff / Quit prompt. See also: *collision*.
_Avoid_: (none — distinct from collision)

**Stripping prefix**:
Portion of a glob `[portal]` key up to but not including the first path component that contains a wildcard character. Removed from the matched source path before appending to the portal's destination value to form the final target path. Defined formally in `spec/dotrift-toml.md` under Path Stripping Rule.

**Profile**:
Named overlay layer of template variables defined as `[profile.<name>]` in `dotrift_data.toml`. Multiple profiles can be active simultaneously; precedence by `activated_at` timestamp — see ADR-0001.
_Avoid_: layer

**Re-import mode**:
`add` subcommand without a `DESTINATION` argument: the destination is derived from the existing database entry for the target file. Used to pull an externally-edited file back into the source tree.

**Pruning**:
`--prune-empty-dirs`: recursive leaf-upward deletion of empty directories after removal operations. Has no upper boundary — the target directory and its ancestors may be pruned. See also: *clean-up*.
_Avoid_: (none — distinct from clean-up)

**Clean-up**:
`--clean-up` on `apply`: removes tracked files that are no longer matched by any portal in the current configuration. Deletes file and database entry if the file is managed; deletes only the database entry if unmanaged or missing. See also: *pruning*.
_Avoid_: (none — distinct from pruning)
