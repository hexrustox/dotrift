# `unapply`

Reverses `apply`, removing managed files from the target. The config must
be loadable (same as `apply`) for target resolution and path validation.

**Usage:** `dotrift unapply [OPTIONS]`

**Options:**

* `--dry-run`: Print planned operations without touching the filesystem or database.
* `--prune-empty-dirs`: Recursively delete orphaned empty directories (see `spec/core.md` Pruning). Stands alone (unlike `apply` where it requires `--clean-up`) because `unapply` inherently removes all managed files.

---

## Execution Pipeline

0. **Load Config:** Same as `apply` Phase 1 (see
   `spec/commands/apply.md#phase-1-state-resolution`) — evaluate
   `dotrift.toml` as template, parse result. Used for target directory
   resolution and to determine which portal entries exist under the current
   config. Only files currently matched by configured portals are eligible
   for unapply.
1. **`--dry-run` Behavior (if active):**
   * Iterate all DB entries. Skip any entry whose target path is not matched by a currently configured portal.
   * Print `[REMOVE] <path>`: File is *managed* (per `spec/core.md#managed-check`) and exists on disk (would be deleted from disk and DB).
   * **Note:** Do not simulate recursive empty directory deletion for `--prune-empty-dirs`; only show the file deletions that would initiate the process.
2. **Execution (if not `--dry-run`):** Iterate all DB entries. Skip any entry whose target path is not matched by a currently configured portal.
   * **Managed File:** Delete from disk. Delete from DB. *Managed* is determined by the shared algorithm at `spec/core.md#managed-check`.
   * **Unmanaged/Missing File:** Do not touch disk. Delete from DB.
3. **Prune Phase:** If `--prune-empty-dirs` is active, prune empty directories (see `spec/core.md` Pruning).