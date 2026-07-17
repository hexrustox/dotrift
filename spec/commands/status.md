# `status`

Reports the management status of the target filesystem.

**Usage:** `dotrift status <SUBCOMMAND> [ARGS]`

**Subcommands:**

* `list [file]`: List all managed files, or check a specific file.
* `clear [file]`: Clear status for a specific file, or all files if omitted.

---

## Execution Pipeline

**`list`:**

1. **Determine Scope:** Specific file or entire DB.
2. **Evaluate Status:** Check if file exists on disk and matches DB entry, using the shared algorithm at `spec/core.md#managed-check` (managed == on-disk state matches DB-recorded state).
3. **Output:** For each entry in scope, evaluate it via the managed check
   (`spec/core.md#managed-check`) and print with the matching prefix:
   * `[MANAGED] target -> source (type)`
   * `[UNMANAGED] target` *(Source path omitted for unmanaged entries.)*

**`clear`:**

1. **Determine Scope:** Specific file or entire DB.
2. **Delete:** Remove entry from DB only. Files on disk are not touched. Remove all entries if no file specified.
