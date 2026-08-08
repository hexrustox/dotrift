# Core

Cross-cutting concepts shared across subcommands: the state database, the
state record, the fingerprint, the managed check, the `active_profiles`
storage, and the state lock. Subcommand specs reference this file rather than
restating these rules.

The control-file formats (`dotrift.toml`, `dotrift_data.toml`,
`.dotriftignore`), the CLI surface, and removal semantics (`--clean-up`,
pruning) are specified in their own documents.

## State database

A single global SQLite database records dotrift's persisted state: one state
record per managed path, plus the active-profile selectors. It is the single
source of truth for the *management state*; comparing it against the target
directory and the desired deployment drives `apply`'s decisions.

* **Location:** `$XDG_STATE_HOME/dotrift/state.sqlite`, falling back to
  `$XDG_DATA_HOME/dotrift/state.sqlite` when `XDG_STATE_HOME` is unset.
  Parent directories are created if they do not exist.
* **Scope:** one database per user, shared across every source directory.
  Records are keyed by absolute target path, so the managed check never needs
  to know which source tree produced a record.
* **Concurrency:** the database is mutated only while holding the state lock
  (see [State lock](#state-lock)). Read-only managed checks may occur without
  the lock, but all writes are serialised by it.

## `managed_paths` Table

One row per managed target path, mirroring the last completed filesystem
action (ADR-0007):

```sql
CREATE TABLE managed_paths (
    target_path  TEXT PRIMARY KEY,
    source_path  TEXT NOT NULL,
    kind         TEXT NOT NULL CHECK (kind IN ('file', 'symlink')),
    link_target  TEXT,
    content_hash TEXT,
    CHECK (kind = 'symlink' AND link_target IS NOT NULL AND content_hash IS NULL
        OR kind = 'file'    AND content_hash IS NOT NULL AND link_target IS NULL)
);
```

* `target_path` — the absolute target path dotrift wrote to. Primary key.
* `source_path` — the source path the entry was deployed from.
* `kind` — whether the deployed path is a file or a symlink. The deploy type
  (symlink, copy, template) is a config concern and is not recorded; the
  fingerprint distinguishes content.
* `link_target` — the fingerprint for a symlink record: the link target of
  the symlink dotrift created.
* `content_hash` — the fingerprint for a file record: an xxHash64 of the
  deployed bytes, hex-encoded.

Directories are never recorded. The CHECK constraint enforces that exactly
one fingerprint column is set, matching the kind.

State mirrors completed filesystem actions: a successful write creates or
updates the record, a successful removal deletes it, and skipped or failed
entries retain their prior records.

## Fingerprint

The recorded last-applied state of a target path dotrift created:

* **Symlink:** the link target of the symlink, as a string.
* **File:** an xxHash64 of the deployed bytes, hex-encoded (for example
  `a1b2c3d4e5f67890`).

Directories have no fingerprint. The fingerprint is compared against the
current on-disk state to decide whether a path is still managed; permissions
are not part of the comparison.

## `active_profiles` Table

Tracks which template profiles are currently active:

```sql
CREATE TABLE active_profiles (
    name         TEXT PRIMARY KEY,
    activated_at INTEGER NOT NULL
);
```

* `name` — the profile name, matching a `[profile.<name>]` section in
  `dotrift_data.toml`. Unique: a profile is either active or not.
* `activated_at` — a monotonic timestamp (milliseconds since the Unix epoch)
  recording when the profile was last activated. Re-activating an already
  active profile updates the timestamp, moving it to the end of the
  precedence order.

Activation and deactivation are performed by the `profile` command (see
`spec/commands/profile.md`). The variable-context precedence algorithm is
defined in `dotrift_data.toml` (Profile Resolution); this table is only its
storage.

## Managed check

The read-only comparison answering "does the on-disk state of a target path
match what the database last recorded dotrift writing there?" It is the
shared logic behind the *managed path* term defined in `CONTEXT.md`.

Given a target path on disk and a record keyed by that path:

1. **No record** — unmanaged.
2. **Kind mismatch** — the on-disk filesystem kind does not match the
   recorded `kind` — unmanaged.
3. **Symlink record:** the on-disk target must be a symlink whose link target
   equals `link_target` — managed; otherwise unmanaged.
4. **File record:** the on-disk target must be a regular file. The current
   bytes are hashed (xxHash64) and compared against `content_hash` — equal is
   managed, otherwise unmanaged. There is no mtime fast-path: the current
   bytes are always hashed.

The check is read-only: it never writes disk or database. Callers decide what
to do with the verdict.

## State lock

An exclusive lock serialising state-database reads and mutations across the
commands that touch the management state: `apply` and the profile commands
(`activate`, `deactivate`). `apply` additionally holds it for its entire
reconcile lifecycle (ADR-0011).

* **Location:** `$XDG_STATE_HOME/dotrift/state.lock`, with the same
  `$XDG_DATA_HOME/dotrift/` fallback as the state database.
* **Mechanism:** `flock` in exclusive, non-blocking mode. The lock is
  released automatically when the process exits, so a crashed invocation can
  never wedge future runs.
* **Scope:** `apply` acquires it before reading the control files and holds
  it through preflight, prompts, filesystem actions, state updates, and exit.
  Short-lived commands acquire it for the duration of their state mutation.
  A concurrent invocation that cannot acquire the lock fails through the
  normal command error path.
