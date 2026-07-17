# `unapply`

Reverses `apply`, removing managed files from the target. The config must
be loadable (same as `apply`) for target resolution and path validation.

**Usage:** `dotrift unapply [OPTIONS]`

**Options:**

* `--dry-run`: Print planned operations without touching the filesystem or database.
* `--prune-empty-dirs`: Recursively delete orphaned empty directories (see spec/core.md section "Pruning"). Stands alone (unlike `apply` where it requires `--clean-up`) because `unapply` inherently removes all managed files.

---

## Execution Pipeline

0. **Load Config:** Use (see spec/commands/apply.md section "Phase 1: State Resolution"). Only files currently matched by configured portals are eligible for unapply.
1. **`--dry-run` Behavior (if active):**
   * Iterate all DB entries. Skip any entry whose target path is not matched by a currently configured portal.
   * Print `[REMOVE] <path>`: File is *managed* (see spec/core.md section "Managed Check") and exists on disk (would be deleted from disk and DB).
   * **Note:** Do not simulate recursive empty directory deletion for `--prune-empty-dirs`; only show the file deletions that would initiate the process.
2. **Execution (if not `--dry-run`):** Iterate all DB entries. Skip any entry whose target path is not matched by a currently configured portal.
   * **Managed File:** Delete from disk. Delete from DB. *Managed* is determined by the shared algorithm (see spec/core.md section "Managed Check").
   * **Unmanaged/Missing File:** Do not touch disk. Delete from DB.
3. **Prune Phase:** If `--prune-empty-dirs` is active, prune empty directories (see spec/core.md section "Pruning").