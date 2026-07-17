# `diff`

Shows a side-by-side diff between a managed target file and its
corresponding source file in the dotrift pager TUI.

**Usage:** `dotrift diff <PATH>`

**Arguments:**

* `<PATH>`: Path to a managed file to diff. If relative, resolved against cwd.

---

## Execution Pipeline

1. **Normalize PATH:** Resolve to absolute path (see `spec/core.md` Path Normalization).
2. **Open DB:** Initialize the database at the standard location (see `spec/core.md` Database).
3. **Look Up PATH:** Query DB for `target_path` == `<PATH>`.
   * *Error:* No entry found — `<PATH>` is not managed by dotrift.
4. **Validate Source:** Check `entry.source_path` exists on disk via `symlink_metadata`. A broken symlink in source is treated as missing.
   * *Error:* Source file `<source_path>` not found.
5. **Validate Target:** Check `<PATH>` exists on disk via `symlink_metadata`. A broken symlink at the target is treated as missing.
   * *Error:* Target file `<PATH>` not found.
6. **Open Pager:** Open the pager TUI in side-by-side diff mode (see `spec/pager.md` Diff Mode).
   * **Left pane:** Target file (the managed file on disk).
   * **Right pane:** Source file (the dotfile in the source directory).