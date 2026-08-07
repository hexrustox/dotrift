# Core

Cross-cutting concepts shared across multiple subcommands: the global CLI
surface, path-handling rules, the database, pruning, and the managed check.
Subcommand specs reference this file rather than restating these rules.

---

## Global CLI Structure

**Usage:** `dotrift [GLOBAL OPTIONS] <COMMAND> [COMMAND ARGS/OPTIONS]`

### Global Options

* `-s, --source <DIR>`: Path to the source directory containing `dotrift.toml` and dotfiles. Default: `~/.local/share/dotrift`.
* `-t, --target <DIR>`: Override the target directory.
* `-c, --config <FILE>`: Override the global config file path. Default: `$XDG_CONFIG_HOME/dotrift/config.toml`.
* `-v, --verbose`: Enable verbose logging.

### Target Directory Precedence

When dotrift needs the target directory it consults, in order:

1. `-t` CLI argument (if provided)
2. `target-directory` in `dotrift.toml` (if provided)
3. `$HOME` environment variable (ultimate fallback). *Error if `$HOME` is unset or empty.*

### Cwd-Relative Resolution

Some CLI arguments accept a path that may be relative. Before such a path is
used as a database key or compared against absolute paths, it is made absolute:

* If the input is already absolute, it is used as-is.
* If the input is relative, it is concatenated with the process's current
  working directory to form an absolute path. Only the process's cwd is
  consulted and symlinks are not followed. The resulting absolute path is then
  handed to Path Normalization.

### Path Normalization

All paths are normalized before use: `./` and `../` components resolved, trailing slashes removed. No tilde (`~`) or environment variable (`$VAR`) expansion. No symlink canonicalization (paths kept logical).

Lexical normalization only is used — `.`/`..` components are resolved as
strings, the filesystem is never consulted, and symlinks are never followed.
The rationale and the invariants this preserves are recorded in
ADR-0003.

### Source Directory Requirement

Commands that read `dotrift.toml` (`apply`, `unapply`, `add`) error if the
source directory does not exist.

---

## Pruning

`--prune-empty-dirs` triggers recursive leaf-upward deletion of empty
directories after a removal operation. The recursion has no upper boundary —
the target directory and its ancestors may be pruned.

`apply` accepts the flag only alongside `--clean-up`; `unapply` accepts it
standalone because every removal it performs is a candidate for pruning. See
each command's spec for the exact integration point.

---

## Database

The local database tracks managed files. It is the single source of truth for resolving states in `diff`, `status`, `apply`, and `unapply`.

**Location:** `$XDG_STATE_HOME/dotrift/db.sqlite` (fallback: `$XDG_DATA_HOME/dotrift.sqlite`). Parent directories are created if they do not exist.

**Format:** SQLite. Single-instance assumption: only one `dotrift` process should access the database at a time. No locking or concurrency guarantees.

### `managed_files` Table

```sql
CREATE TABLE managed_files (
    target_path TEXT PRIMARY KEY,
    deploy_type TEXT NOT NULL,
    source_path TEXT NOT NULL,
    hash TEXT,
    symlink_target TEXT,
    mtime INTEGER
);
```

**Columns:**

* `target_path`: Absolute path of the managed file (primary key).
* `deploy_type`: Enum (`symlink` | `copy` | `tmpl`).
* `source_path`: Absolute path in the source directory. Used to read content for copies, or verify link targets for symlinks.
* `hash`: Hex digest using `xxHash64` of target file content at last apply. NULL for symlinks. Used to detect external modifications to the target (managed check compares target-on-disk hash against DB hash).
* `symlink_target`: `read_link(source_path)` if source is a symlink and deploy type is `copy`, NULL otherwise. Decouples managed check from current source filesystem state.
* `mtime`: Modification time of the target file at last apply, stored as milliseconds since Unix epoch. NULL for symlinks. When the on-disk mtime matches this value, the file is considered managed without computing the hash.

### `active_profiles` Table

Tracks which template profiles are currently active.

```sql
CREATE TABLE IF NOT EXISTS active_profiles (
    activated_at INTEGER NOT NULL,
    name         TEXT NOT NULL UNIQUE
);
```

**Columns:**

* `activated_at`: Unix timestamp in milliseconds when the profile was activated. Last-activated (highest timestamp) wins on variable conflict.
* `name`: Profile name, matches a `[profile.<name>]` section in `dotrift_data.toml`.

The precedence algorithm lives elsewhere; this table is only its storage. (see spec/dotrift-data-toml.md section "Profile Resolution")

---

## Managed Check

The managed check answers "does the on-disk state of a target path match what
the database last recorded dotrift writing there?" It is the shared logic
behind the *managed* term defined in `CONTEXT.md`.

Given a target path on disk and a database entry keyed by that path:

1. **No DB entry** → unmanaged.
2. **DB `deploy_type` = `symlink`:** the on-disk target must be a symlink whose link target equals `DB.source_path` → managed; otherwise unmanaged.
3. **DB `deploy_type` = `copy` with non-NULL `symlink_target`:** the on-disk target must be a symlink pointing to `DB.symlink_target` → managed; otherwise unmanaged.
4. **DB `deploy_type` = `copy` or `tmpl` with stored `hash`:** the on-disk target must be a regular file. If `DB.mtime` is non-NULL and matches the on-disk mtime → managed (the mtime fast-path skips the hash). Otherwise, fall back to computing the on-disk hash and comparing against `DB.hash`: equal → managed, else unmanaged.
5. Any other state → unmanaged.

The check is read-only: it never writes disk or DB. Callers decide what to do
with the managed/unmanaged verdict (see each command's pipeline).
