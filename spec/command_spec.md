# Dotrift Command Specification

This document defines the CLI structure, global behaviors, database schema, and specifications for all `dotrift` subcommands. The manager evaluates configurations defined in `dotrift.toml`.

## Global CLI Structure

**Usage:** `dotrift [GLOBAL OPTIONS] <COMMAND> [COMMAND ARGS/OPTIONS]`

### Global Options
* `-s, --source <DIR>`: Path to the source directory containing `dotrift.toml` and dotfiles. Default: `~/.local/share/dotrift`.
* `-t, --target <DIR>`: Override the target directory. 

**Target Directory Precedence:**
1. `-t` CLI argument (if provided)
2. `target-dir` in `dotrift.toml` (if provided)
3. `$HOME` environment variable (ultimate fallback)

---

## Database Schema

The local database tracks managed files. It is the single source of truth for resolving states in `diff`, `status`, `apply`, and `unapply`.

**Location:** `~/.local/state/dotrift.db` (global).

**Format:** SQLite.

**Schema:**
```sql
CREATE TABLE entries (
    target_path TEXT PRIMARY KEY,
    action_type TEXT NOT NULL CHECK(action_type IN ('symlink', 'copy')),
    reference TEXT NOT NULL,
    hash TEXT
);
```

**Columns:**
* `target_path`: Absolute path of the managed file (primary key).
* `action_type`: Enum (`symlink` | `copy`).
* `reference`: Absolute path in `source-dir`. Used to read content for copies, or verify link targets for symlinks.
* `hash`: Hex digest using `xxHash` of source content at last apply. NULL for symlinks.

---

## `apply` Command

Evaluates `dotrift.toml` and applies the defined state to the target filesystem.

**Usage:** `dotrift apply [OPTIONS]`
**Options:**
* `--dry-run`: Print planned operations without touching the filesystem or database.
* `--clean-up`: Remove previously managed files no longer mapped in `dotrift.toml`.
* `--prune-empty-dirs`: Recursively delete orphaned empty directories. Requires `--clean-up`.

### Execution Pipeline

#### Phase 1: State Resolution
1. Parse `dotrift.toml`, resolve target dir, normalize `./` prefixes.
2. **Portal Resolution:** Glob `[portal]` keys against source files/dirs. Calculate `target_path` via Stripping Rule. Drop matches hitting `ignore`. Store in `HashMap<TargetPath, FileIntent>`. A source file/dir can match multiple portals if no target collision.
  * *Error:* Target path collision. Show both source paths and the collision target.
3. **Rule Application:** Apply `[rule]` in order, shallow-merging properties to determine final `type` and `mode`.
  * *Warning:* `mode` set on `type = "symlink"` (ignored during execution).

#### Phase 2: Tree Construction
Build a Rose Tree from the HashMap. 
1. Initialize an empty Root Node.
2. For each `target_path` in the HashMap, split the path by `/`.
3. Traverse the tree from the root, component by component:
  * For parent components: Create Directory Nodes if absent. "Cd" into them.
  * For the final component: Create a File Node, attaching the `FileIntent` from the HashMap.
4. *Error:* If the builder encounters a File Node where a Directory Node is required (or vice versa) within the planned tree, throw a structural Error and halt.

#### `--dry-run` Behavior
If active, the command executes Phases 1 and 2, then simulates the remaining phases to generate a report, and finally exits without modifying the filesystem or database.

The plan is printed to `stdout` and includes:

1.  **Deployment Plan:** The structured tree of files to be created or updated is printed, indicating the intended action for each.
    *   `[CREATE] /path/to/new/file (symlink)`
    *   `[UPDATE] /path/to/existing/file (copy)`

2.  **Clean-up Plan (if `--clean-up` is also passed):** After printing the deployment plan, the dry run will simulate the clean-up phase. It iterates the database and, for any path not found in the planned tree (from Phase 2), it checks its on-disk status and prints one of the following:
    *   `[WOULD CLEAN] <path>`: For a file that is currently managed and exists on disk, which would be deleted.
    *   `[WOULD UNTRACK] <path>`: For a database entry that is stale (file is missing) or points to a modified file (unmanaged), where only the database entry would be removed.

**Note:** The dry run will not simulate the recursive deletion of empty directories (`--prune-empty-dirs`), as this cannot be determined without modifying the filesystem. It will only show the files whose deletion would initiate the process.

#### `--clean-up` Behavior
If active, execute after Phase 2, before Phase 3.
Iterate DB. If a path is NOT in the Phase 2 tree:
1. Check if file exists on disk and matches DB (Symlink check or hash check).
2. **Managed:** Delete file, delete from DB. If `--prune-empty-dirs`, bubble up empty dir deletion. Print `[CLEANED]`.
3. **Unmanaged/Missing:** Do not touch disk. Delete from DB. Print `[UNMANAGED]` or `[STALE DB]`.

#### Phase 3: Execution
Traverse Rose Tree top-down (Pre-order DFS).

**Directory Nodes:**
* `fs::create_dir`. If fails due to existing file -> Collision prompt (`[s]kip / [o]verwrite / [q]uit`). Non-interactive (non-TTY): default `[s]kip`.
  * **skip:** Abort subtree.
  * **overwrite:** Delete file then creates dir, deletes DB entry to avoid stale state, continue children.
  * **quit:** Halt program.
* Other errors abort subtree.

**File Nodes:**
1. **Exists?** No -> Write immediately (skip DB checks).
2. **Exists?** Yes -> A Directory:
  * Halt and prompt (`[s]kip / [o]verwrite / [q]uit`). Non-interactive (non-TTY): default `[s]kip`.
    * **skip:** Do not touch the filesystem. Continue traversal.
    * **overwrite:** Delete the directory recursively, delete DB entries under the directory. Proceed to writing.
    * **quit:** Immediately terminate the program.
3. **Exists?** Yes -> Determine Management Status:
  * Unmanaged if: Not in DB, DB type differs from disk, symlink target differs, or DB hash differs from target hash.
4. **Action:**
  * **Managed:** Overwrite silently.
  * **Unmanaged:** Halt and prompt (`[s]kip / [o]verwrite / [q]uit`). Non-interactive (non-TTY): default `[s]kip`.
    * **skip:** Do not touch the filesystem. Continue traversal.
    * **overwrite:** Delete the disk entity. Proceed to writing.
    * **quit:** Immediately terminate the program.
5. **Write:** If the file is managed AND (symlink target == DB.reference OR copy hash(target) == hash(source)), skip. Else `copy` -> copy + chmod, `symlink` -> explicit `unlink` then `symlink`.

#### Phase 4: Database Synchronization
Insert/update DB entry immediately after each successful write during Phase 3 traversal. On quit, DB already reflects all successful operations.

---

## `unapply` Command

Reverses the `apply` process, removing managed files from the target.

**Usage:** `dotrift unapply [OPTIONS]`

**Options:**
* `--dry-run`: Print planned operations without touching the filesystem or database.
* `--prune-empty-dirs`: Recursively delete orphaned empty directories.

### Execution Pipeline

1. **`--dry-run` Behavior (if active):**
   * Iterate all DB entries.
   * Print to `stdout`:
     * `[WOULD REMOVE] <path>`: File is managed and exists on disk (would be deleted from disk and DB).
     * `[WOULD UNTRACK] <path>`: File is unmanaged/modified or missing (would only be removed from DB).
   * **Note:** Do not simulate recursive empty directory deletion for `--prune-empty-dirs`; only show the file deletions that would initiate the process.

2. **Execution (if not `--dry-run`):** Iterate all DB entries.
   * **Managed File:** Delete from disk. Delete from DB.
   * **Unmanaged/Missing File:** Do not touch disk. Delete from DB.

3. **Prune Phase:** If `--prune-empty-dirs` is active, after all file removals, recursively delete empty directories starting from leaf directories upward.

---

### `add` Command
**Usage:** `dotrift add <TARGET_FILE> <SOURCE_RELATIVE_PATH>`

**Arguments:**
* `<TARGET_FILE>`: Absolute path to existing file on disk.
* `<SOURCE_RELATIVE_PATH>`: Absolute path in `source-dir`.

### Execution Pipeline
1. **Validate:** Ensure `<TARGET_FILE>` exists.
2. **Resolve Source:** Join `source-dir` + `<SOURCE_RELATIVE_PATH>`.
3. **Collision Check:** Error if resolved source path already exists on disk.
4. **Clone File:** Create source parent dirs. Recursively copy target (dirs/symlinks treated as files, copying symlink itself) to source path.
5. **Open Editor:** Run `($VISUAL/$EDITOR) <source-dir>/dotrift.toml` to let user add a portal entry if needed, if no editor command found from the env vars, skip.

---

## `diff` Command

Prints the content differences between the source file and the target file.

**Usage:** `dotrift diff <TARGET_FILE> [OPTIONS]`

**Arguments:**
* `<TARGET_FILE>`: Absolute path to a specific file to check.
* `[OPTIONS]`: Optional. Flags passed to the `diff` command.

### Execution Pipeline

1. **Determine Scope:** Specific file (error if not exist on disk; OK if in DB but modified).
2. **Filter:** Skip if managed.
3. **Execute Diff:** Run `diff <source> <TARGET_FILE> [OPTIONS]`, source is `DB.reference`.

---

## `status` Command

Reports the management status of the target filesystem.

**Usage:** `dotrift status [TARGET_FILE]`

**Arguments:**
* `[TARGET_FILE]`: Optional. Specific file to check. If omitted, lists all managed files.

### Execution Pipeline

1. **Determine Scope:** Specific file (print `[UNMANAGED]` if not in DB) or entire DB.
2. **Evaluate Status:** Use standard Management Status logic.
3. **Output:**
  * `[MANAGED]   <path> (<type>)`
  * `[MODIFIED]  <path> (<type>)` *(In DB but unmanaged: type/link/hash mismatch)*
  * `[MISSING]   <path> (<type>)` *(In DB, missing on disk)*
  * `[UNMANAGED] <path>` *(Not in DB)*
