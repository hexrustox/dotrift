# ADR-0015: No signal handling and no database migrations

dotrift installs no signal handlers. SIGINT and SIGTERM kill the process at an
arbitrary instruction point, exactly as the platform default dictates. The
spec's earlier claim that "external interruption follows the same per-action
handling" was aspirational and is withdrawn: an interrupted deploy action can
leave a file on disk with no state record, or an obstruction removed with its
replacement unwritten. This is acceptable because the state model already
treats such residue safely — a file without a matching record is an ordinary
untracked obstruction, surfaced by the next `apply` (or by `status`), never
silently overwritten. Handled signals would buy clean stops at action
boundaries at the cost of async-signal-safety discipline through SQLite and the
filesystem walk, which is not worth it for a CLI whose worst case is one
re-run.

The state database likewise carries no schema version and no migration path.
There is no `PRAGMA user_version`, no migration table, no upgrade step. A
database file that fails to open, fails to parse as SQLite, or lacks the
expected tables is a hard error in every command that touches it — `apply`,
`profile activate`, `profile deactivate`, `profile list`, `profile show`,
`status` — with no quarantine, no recreate, and no repair. A missing file is
the only absence treated as empty state. Recreating a corrupt database
automatically was rejected: every record would silently become an untracked
obstruction, converting a repairable error into unprompted data-loss risk. The
user deletes the file by hand if they want a fresh start.

Both decisions share one principle: dotrift never silently reinterprets state
it did not write. Interrupted runs leave verifiable residue; corrupt databases
refuse to run. The next invocation either finds the world as the last completed
step left it, or stops and says so.

## Amendment (2026-09): partial schemas are completed on open

The paragraph above says a database that "lacks the expected tables is a hard
error". That letter is superseded: an existing database file missing one of the
expected tables is completed on open (`CREATE TABLE IF NOT EXISTS`), not
rejected. The hard-error rule now covers only files that cannot be opened or
parsed as SQLite. Creating a missing table cannot reinterpret any record
dotrift wrote; the data-loss risk this ADR guards against arises only when
existing rows are reinterpreted or silently discarded, which schema completion
never does. `spec/core.md § State database` carries the current contract.
