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
  `$HOME/.local/state/dotrift/state.sqlite` when `XDG_STATE_HOME` is unset,
  and to `$XDG_DATA_HOME/dotrift/state.sqlite` when neither the state
  directory nor a home directory can be resolved. An error is raised only
  when no state location can be resolved at all.
* **Creation:** opening the database creates the state directory and an empty
  database file when absent. Every command that opens the state database —
  including `apply --dry-run` — therefore leaves these artifacts. Creation is
  not a deployment action and touches nothing under the target directory.
* **Scope:** one database per user, shared across every source directory.
  Records are keyed by absolute target path, so the managed check never needs
  to know which source tree produced a record.
* **Concurrency:** the database is mutated only while holding the state lock
  (see [State lock](#state-lock)). Read-only managed checks may occur without
  the lock, but all writes are serialised by it.
* **Corruption:** a missing database is a valid empty state. A database that
  cannot be opened or parsed as SQLite is a hard error in every command that
  touches it: no quarantine, no recreate, no repair. An existing database
  missing an expected table is completed on open (`CREATE TABLE IF NOT
  EXISTS`) rather than rejected; only a file that cannot be opened or parsed
  fails (see the amendment to ADR-0015). There is no schema version and no
  migration path; the user deletes the file by hand to start fresh.

## managed_paths Table

One row per managed target path, mirroring the last completed filesystem
action (ADR-0007):

```sql
CREATE TABLE managed_paths (
    target_path  TEXT PRIMARY KEY,
    source_path  TEXT NOT NULL,
    kind         TEXT NOT NULL CHECK (kind IN ('file', 'symlink')),
    content_hash TEXT,
    CHECK (kind = 'symlink' AND content_hash IS NULL
        OR kind = 'file'    AND content_hash IS NOT NULL)
);
```

* `target_path` — the absolute target path dotrift wrote to. Primary key.
* `source_path` — the source path the entry was deployed from; also the
  fingerprint for a symlink record, whose on-disk link target is that path.
* `kind` — whether the deployed path is a file or a symlink. The deploy type
  (symlink, copy, template) is a config concern and is not recorded; the
  fingerprint distinguishes content.
* `content_hash` — the fingerprint for a file record (see
  [Fingerprint](#fingerprint)).

Directories are never recorded. The CHECK constraint enforces that a file
record carries `content_hash` while a symlink record never does.

State mirrors completed filesystem actions: a successful write creates or
updates the record, a successful removal deletes it, and skipped or failed
entries retain their prior records.

## Fingerprint

The recorded last-applied state of a target path dotrift created:

* **Symlink:** the source path the entry was deployed from, which is the
  symlink's on-disk link target, as a string.
* **File:** an xxHash64 (seed 0) of the deployed bytes, hex-encoded as
  exactly 16 lowercase hexadecimal digits (for example `a1b2c3d4e5f67890`),
  computed by streaming the content.

Directories have no fingerprint. The fingerprint is compared against the
current on-disk state to decide whether a path is still managed; permissions
are not part of the comparison, and there is no mtime fast-path: the current
bytes are always hashed.

## active_profiles Table

Tracks which template profiles are currently active:

```sql
CREATE TABLE active_profiles (
    name         TEXT PRIMARY KEY,
    activated_at INTEGER NOT NULL
);
```

* `name` — the profile name, matching a `[profile.<name>]` section in
  `dotrift_data.toml`. Unique: a profile is either active or not.
* `activated_at` — milliseconds since the Unix epoch. dotrift forces the
  value strictly upward: an activation is stored greater than every value
  already stored, so re-activating an already-active profile moves it to the
  end of the precedence order and two activations can never tie. A stored
  value can therefore exceed the wall clock. The lexicographic tie-break in
  Profile Resolution remains as a guard for state written outside dotrift.

Rows are read in ascending `activated_at` order, ties broken lexicographically
by name. Activation and deactivation are performed by the `profile` command
(see `spec/commands/profile.md`). The variable-context precedence algorithm is
defined in `spec/dotrift-data-toml.md § Profile resolution`; this table is
only its storage.

## Managed check

The read-only comparison answering "does the on-disk state of a target path
match what the database last recorded dotrift writing there?" It is the
shared logic behind the *managed path* term defined in `spec/CONTEXT.md`.

Given a target path on disk and a record keyed by that path:

1. **No record** — unmanaged.
2. **Kind mismatch** — the on-disk filesystem kind does not match the
   recorded `kind` — unmanaged.
3. **Symlink record:** the on-disk target must be a symlink whose link target
   equals the recorded `source_path` — managed; otherwise unmanaged.
4. **File record:** the on-disk target must be a regular file. The current
   bytes are hashed (xxHash64, seed 0) and compared against `content_hash` —
   equal is managed, otherwise unmanaged. There is no mtime fast-path: the
   current bytes are always hashed.
5. **Read failure** — a failure to read the on-disk metadata (the filesystem
   kind) or the file content (for hashing) yields unmanaged: the current
   state cannot be verified against the record, so the path is not managed.

The check is read-only: it never writes disk or database. Callers decide what
to do with the verdict.

## State lock

An exclusive lock serialising state-database mutations across the commands
that touch the management state: `apply` and the profile commands
(`activate`, `deactivate`). Read-only commands (`status`, `profile list`,
`profile show`) read the database without the lock.

* **Location:** `$XDG_STATE_HOME/dotrift/state.lock`, resolved like the state
  database. Acquiring it creates the lock file when absent.
* **Mechanism:** `flock` in exclusive, non-blocking mode. The kernel releases
  the lock when the process exits, so a crashed invocation can never wedge
  future runs.
* **Scope:** `apply` acquires it before reading the control files and holds
  it for the entire run — through preflight, prompts, filesystem actions,
  state updates, and the summary — releasing it when the run completes.
  Short-lived commands acquire it for the duration of their state mutation.
  A concurrent invocation that cannot acquire the lock fails through the
  normal command error path (ADR-0011).
