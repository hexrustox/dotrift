# `status`

Reports the management state: one line per state record in the
`managed_paths` table, showing the recorded kind and whether the target path
still passes the managed check. `status` is read-only: it reads the state
database and the target filesystem but never writes either, and it does not
read the control files or resolve a desired deployment. The managed check is
defined in `core.md` (Managed check); the storage schema is defined in
`core.md` (`managed_paths` Table); the global CLI conventions are defined in
`global.md`.

**Usage:** `dotrift status`

`status` takes no arguments and no subcommands.

## Pipeline

1. Open the state database (see `core.md` State database). A missing database
   is a valid empty state with no records.
2. Query every row from the `managed_paths` table.
3. For each record, run the managed check against the target path on disk,
   using the recorded kind and fingerprint. No special handling applies:
   whatever verdict the check yields is the verdict `status` reports, whether
   the target is missing, unreadable, changed, or a different kind.
4. Print one line per record, sorted lexicographically by target path.

## Output

Each record prints one line:

```
managed    symlink  ~/.zshrc <- dotfiles/.zshrc
unmanaged  file     ~/.config/nvim/init.lua <- dotfiles/config/nvim/init.lua
```

- The verdict is `managed` when the managed check passes and `unmanaged`
  otherwise; the verdict word is colored green or red respectively (see
  `global.md` § Output conventions).
- The kind is the *recorded* kind (`file` or `symlink`), even when the on-disk
  kind no longer matches.
- The target path is the recorded target path, displayed per the path display
  convention (`global.md` § Output conventions).
- The source path is the recorded path the target maps from — the entry's path
  inside the source directory, or the link destination for a symlink deploy —
  displayed the same way.

Lines are sorted lexicographically by target path. An empty database, or a
database with no records, prints nothing.

## Global options

`status` accepts the global `-s`/`-t` options but treats them as no-ops: it
reads no control files and resolves no paths from configuration, so the
global source-directory requirement (see `global.md` Source directory
requirement) does not apply.

## Concurrency

`status` is read-only — it performs only database reads and read-only managed
checks — so it never acquires the state lock.

## Exit status

`status` always succeeds. It is a report, not a check: an unmanaged verdict
never makes the run unsuccessful.
