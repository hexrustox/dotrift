# `apply`

Evaluates `dotrift.toml` and applies the defined state to the target
filesystem.

**Usage:** `dotrift apply [OPTIONS]`

**Options:**

* `--dry-run`: Print planned operations without touching the filesystem or database.
* `--clean-up`: Remove previously managed files no longer mapped in `dotrift.toml`.
* `--prune-empty-dirs`: Recursively delete orphaned empty directories (see `spec/core.md` Pruning). Requires `--clean-up`.

---

## Execution Pipeline

### Phase 1: State Resolution

<a id="phase-1-state-resolution"></a>

1. **Template Evaluation:** Load `dotrift_data.toml` from source dir (if present), query DB for active profiles, resolve template context per `spec/dotrift-data-toml.md` Profile Resolution. Read `dotrift.toml`, evaluate as a template (see `spec/templater.md`), then parse the result as TOML config. Template errors (parse or render) are fatal.
2. **Portal Resolution:** Glob `[portal]` keys against source files/dirs. Calculate `target_path` via the Path Stripping Rule (`spec/dotrift-toml.md`). Filter using the `ignore` list (`spec/dotrift.toml`). Store in `HashMap<TargetPath, (SourcePath, DeployType, Option<Mode>)>`. A source file/dir can match multiple portals if no target collision.
   * Literal directory keys expand to one entry per contained file (not one entry for the directory itself).
   * *Error:* Target path collision (see `spec/dotrift-toml.md` Validation & Errors). Show all colliding source paths and the collision target. Halt program.
   * *Error:* Resolved target path is inside `source-dir`. Show the target path and source path. Halt program.
3. **Rule Application:** Apply `[rule]` in order, shallow-merging properties to determine final `type` and `mode` (see `spec/dotrift-toml.md` `[rule]`). Last rule wins on conflict.

### Phase 2: Tree Construction

Build a Rose Tree from the HashMap.

1. Initialize an empty Root Node.
2. For each `target_path` in the HashMap, split the path by `/`.
3. Traverse the tree from the root, component by component:
   * For parent components: Create Directory Nodes if absent. "Cd" into them.
   * For the final component: Create a File Node, attaching the `(SourcePath, DeployType, Option<Mode>)` tuple from the HashMap.
4. *Error:* If the builder encounters a File Node where a Directory Node is required (or vice versa) within the planned tree, throw a structural error and halt.

### `--dry-run` Behavior

If active, execute Phases 1 and 2, then simulate the remaining phases to
generate a report, and exit without modifying the filesystem or database.

The plan is printed and includes:

1. **Clean-up Plan (if `--clean-up` is also passed):** The dry run simulates the clean-up phase first. It iterates the database and prints `[REMOVE] <path>` for any entry not found in the Phase 1 portal entries HashMap.
2. **Deployment Plan:** The structured tree of files to be created is printed.
   * `[CREATE] /path/to/file (symlink)`
   * `[CREATE] /path/to/file (file)`
   * `[CREATE] /path/to/file (tmpl)`

The dry run does not simulate recursive deletion of empty directories
(`--prune-empty-dirs`).

### `--clean-up` Behavior

If active, execute after Phase 1, before Phase 2. If `--dry-run` is also
active, print planned removals without modifying FS or DB. Otherwise iterate
the database. For each entry whose path is NOT in the Phase 1 portal entries
HashMap:

1. Check if file exists on disk and matches DB (symlink check or hash check).
2. **Managed:** Delete file, delete from DB. If `--prune-empty-dirs` is active, prune empty directories (see `spec/core.md` Pruning).
3. **Unmanaged/Missing:** Do not touch disk. Delete from DB.

The distinction between this clean-up operation and pruning is recorded in
`CONTEXT.md` (Clean-up vs Pruning).

### Phase 3: Execution

Traverse the Rose Tree top-down (Pre-order DFS).

#### Directory Nodes

`fs::create_dir_all`. If it fails due to an existing non-directory (file,
symlink [even if pointing to a directory], socket, etc.), the collision
prompt is shown (see `spec/prompt.md`). Per-option actions:

* **skip:** Abort subtree.
* **overwrite:** Delete file, create dir, delete DB entry to avoid stale state, continue children.
* **quit:** Halt program.
* **diff:** Open pager in single-pane mode, displaying the obstructing file's content with a header noting the directory that dotrift needs to create.

Other errors abort the subtree.

#### File Nodes

1. **Exists?** No → proceed to step 4 (write).
2. **Exists? Yes → a Directory:** Collision prompt (see `spec/prompt.md`).
   * **skip:** Do not touch the filesystem. Continue traversal.
   * **overwrite:** Delete the directory recursively, delete DB entries under the directory. Proceed to step 4.
   * **quit:** Immediately terminate the program.
   * **diff:** Open pager in explorer mode. Left pane: file browser at the target directory. Right pane: source file content.
3. **Exists? Yes → File or symlink on disk:**
   a. **Identical Check:**
<a id="identical-check"></a>

      Determine if the target matches what dotrift would write this run,
      irrespective of DB state. The *identical* term is defined in
      `CONTEXT.md`; the per-deploy-type rules:

      - `symlink`: target is a symlink AND link target == `entry.source` → identical.
      - `copy` where source is regular file: target is a regular file AND hash of source file content equals hash of target file content on disk → identical.
      - `copy` where source is symlink: target is a symlink AND link target == `read_link(entry.source)` → identical.
      - `tmpl`: never identical. Always proceed to management check.

      If identical: consult the global `overwrite-identical` setting (see
      `spec/global-config.md`). If set, update the DB entry. Skip write. Return.

   b. **Management Check:** Determine whether the target is unchanged since
      dotrift last wrote it, using the shared algorithm at
      `spec/core.md#managed-check`. The verdict drives the action below.

   c. **Action:**
      - **Managed:** Proceed to step 4 (write) silently. Safe to overwrite — no external modification detected.
      - **Unmanaged:** Collision prompt (see `spec/prompt.md`).
        - **skip:** Do not touch the filesystem. Continue traversal.
        - **overwrite:** Delete the disk entity. Proceed to step 4.
        - **quit:** Immediately terminate the program.
        - **diff:** Open pager in side-by-side mode. Left pane: target file on disk. Right pane: source file.

4. **Write:** The conceptual behavior of each deploy type when the source is
   a symlink is specified at `spec/dotrift-toml.md#source-symlink-behavior`.
   The concrete steps:

   - `symlink`: unlink target (if exists) → `symlink(entry.source, target)`.
   - `copy`: if `entry.source` is a symlink on disk, create a symlink at target pointing to `read_link(entry.source)`. Otherwise, `fs::copy(source, target)`. After write: if `mode` set AND target is regular file → `chmod`.
   - `tmpl`: resolve template context from `dotrift_data.toml` (see `spec/dotrift-data-toml.md`). If source is a symlink, resolve it first. Parse the source file as a template (see `spec/templater.md`), evaluate with context, write rendered output to target via streaming writer. After write: if `mode` set AND target is regular file → `chmod`.

5. **DB Sync:** Insert/update DB entry after every successful write. The
   schema is specified in `spec/core.md`. Fields written:

   - `target_path`: absolute target path.
   - `deploy_type`: `symlink`, `copy`, or `tmpl` (from rule).
   - `source_path`: absolute source path (from portal entry).
   - `hash`: hex digest of target file content if source resolves to a regular file (same hash compared during Identical and Managed checks), NULL if source is a symlink or deploy type is `symlink`. For `tmpl`, hash is computed on the **rendered** target file.
   - `symlink_target`: `read_link(source_path)` if source is a symlink and deploy type is `copy`, NULL otherwise.
   - `mtime`: modification time of the target file after write, read via `symlink_metadata().modified()`, stored as milliseconds since Unix epoch. NULL for symlinks.