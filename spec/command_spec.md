# Dotrift Command Specification

This document defines the CLI structure, global behaviors, database schema, and specifications for all `dotrift` subcommands. The manager evaluates configurations defined in `dotrift.toml`.

## Global CLI Structure

**Usage:** `dotrift [GLOBAL OPTIONS] <COMMAND> [COMMAND ARGS/OPTIONS]`

### Global Options
* `-s, --source <DIR>`: Path to the source directory containing `dotrift.toml` and dotfiles. Default: `~/.local/share/dotrift`.
* `-t, --target <DIR>`: Override the target directory. 

**Target Directory Precedence:**
1. `-t` CLI argument (if provided)
2. `target-directory` in `dotrift.toml` (if provided)
3. `$HOME` environment variable (ultimate fallback). *Error if `$HOME` is unset or empty.*

**Path Normalization:**
All paths are normalized before use: `./` and `../` components resolved, trailing slashes removed. No tilde (`~`) or environment variable (`$VAR`) expansion. No symlink canonicalization (paths kept logical).

**Source Directory Requirement:**
Commands that read `dotrift.toml` (`apply`, `unapply`, `add`) error if the source directory does not exist.

### Global Configuration

Loaded from `$XDG_CONFIG_HOME/dotrift/config.toml` (fallback: `$XDG_DATA_HOME/dotrift/config.toml`). This file is optional; missing files are treated as empty (all defaults apply). Invalid or malformed TOML is an error. Partial configs merge with defaults.

```toml
overwrite-identical = false

[editor-command]
command = "vim"
args = ["-f"]
```

**Fields:**
* `overwrite-identical` (bool): Whether to update the DB entry when a target file already matches what dotrift would write. Default: `false`.
* `editor-command` (optional table): Command to open `dotrift.toml` for the `add` command.
  * `command` (string): The executable name or path.
  * `args` (array of strings): Arguments passed to the command.

---

## Database Schema

The local database tracks managed files. It is the single source of truth for resolving states in `diff`, `status`, `apply`, and `unapply`.

**Location:** `$XDG_STATE_HOME/dotrift/db.sqlite` (fallback: `$XDG_DATA_HOME/dotrift.sqlite`). Parent directories are created if they do not exist.

**Format:** SQLite. Single-instance assumption: only one `dotrift` process should access the database at a time. No locking or concurrency guarantees.

**Schema:**
```sql
target_path TEXT PRIMARY KEY,
deploy_type TEXT NOT NULL,
source_path TEXT NOT NULL,
hash TEXT
```

**Columns:**
* `target_path`: Absolute path of the managed file (primary key).
* `deploy_type`: Enum (`symlink` | `copy`).
* `source_path`: Absolute path in `source-dir`. Used to read content for copies, or verify link targets for symlinks.
* `hash`: Hex digest using `xxHash64` of **source** file content at last apply. NULL for symlinks. Used to detect external modifications to the target (managed check compares target-on-disk hash against DB hash).

---

## `apply` Command

Evaluates `dotrift.toml` and applies the defined state to the target filesystem.

**Usage:** `dotrift apply [OPTIONS]`
**Options:**
* `--dry-run`: Print planned operations without touching the filesystem or database.
* `--clean-up`: Remove previously managed files no longer mapped in `dotrift.toml`.
* `--prune-empty-dirs`: Recursively delete orphaned empty directories. No upper boundary — may prune the target directory itself or its ancestors. Requires `--clean-up`.

### Execution Pipeline

#### Phase 1: State Resolution
1. Parse `dotrift.toml`, resolve target dir, normalize paths (see Global Options).
2. **Portal Resolution:** Glob `[portal]` keys against source files/dirs. Calculate `target_path` via Stripping Rule. Filter against `ignore` list in order — `!` patterns re-include previously excluded paths (gitignore-style). Store in `HashMap<TargetPath, (SourcePath, DeployType, Option<Mode>)>`. A source file/dir can match multiple portals if no target collision.
   * Literal directory keys expand to one entry per contained file (not one entry for the directory itself).
   * *Error:* Target path collision. Show all colliding source paths and the collision target. Halt program.
3. **Rule Application:** Apply `[rule]` in order, shallow-merging properties to determine final `type` and `mode`. Last rule wins on conflict.
  * *Warning:* `mode` set on `type = "symlink"` (ignored during execution).

#### Phase 2: Tree Construction
Build a Rose Tree from the HashMap. 
1. Initialize an empty Root Node.
2. For each `target_path` in the HashMap, split the path by `/`.
3. Traverse the tree from the root, component by component:
  * For parent components: Create Directory Nodes if absent. "Cd" into them.
  * For the final component: Create a File Node, attaching the `(SourcePath, DeployType, Option<Mode>)` tuple from the HashMap.
4. *Error:* If the builder encounters a File Node where a Directory Node is required (or vice versa) within the planned tree, throw a structural Error and halt.

#### `--dry-run` Behavior
If active, the command executes Phases 1 and 2, then simulates the remaining phases to generate a report, and finally exits without modifying the filesystem or database.

The plan is printed to `stdout` and includes:

1.  **Deployment Plan:** The structured tree of files to be created is printed.
    *   `[CREATE] /path/to/file (symlink)`
    *   `[CREATE] /path/to/file (file)`

2.  **Clean-up Plan (if `--clean-up` is also passed):** After printing the deployment plan, the dry run simulates the clean-up phase. It iterates the database and prints `[REMOVE] <path>` for any entry not found in the planned tree.

**Note:** The dry run does not simulate recursive deletion of empty directories (`--prune-empty-dirs`).

#### `--clean-up` Behavior
If active, execute after Phase 2, before Phase 3. If `--dry-run` is also active, print planned removals without modifying FS or DB. Otherwise:
Iterate DB. If a path is NOT in the Phase 2 tree:
1. Check if file exists on disk and matches DB (symlink check or hash check).
2. **Managed:** Delete file, delete from DB. If `--prune-empty-dirs`, bubble up empty dir deletion (no upper boundary).
3. **Unmanaged/Missing:** Do not touch disk. Delete from DB.

#### Phase 3: Execution
Traverse Rose Tree top-down (Pre-order DFS).

**Directory Nodes:**
* `fs::create_dir_all`. If fails due to existing non-directory (file, symlink [even if pointing to a directory], socket, etc.) -> Collision prompt (`[s]kip / [o]verwrite / [q]uit`).
  * **skip:** Abort subtree.
  * **overwrite:** Delete file then creates dir, deletes DB entry to avoid stale state, continue children.
  * **quit:** Halt program.
* Other errors abort subtree.

**Prompt Behavior:** If stdin is not a TTY, all prompts default to `skip` to avoid hanging in piped or automated contexts.

**File Nodes:**
1. **Exists?** No → Proceed to step 4 (write).
2. **Exists?** Yes → A Directory:
  * Halt and prompt (`[s]kip / [o]verwrite / [q]uit`). Prompt loops until valid input.
    * **skip:** Do not touch the filesystem. Continue traversal.
    * **overwrite:** Delete the directory recursively, delete DB entries under the directory. Proceed to step 4.
    * **quit:** Immediately terminate the program.
3. **Exists?** Yes → File or symlink on disk:
a. **Identical Check:** Determine if the target matches what dotrift would write.
      - `symlink`: target is symlink AND link target == `entry.source` → identical.
      - `copy` where source is regular file: target is regular file AND hash of source file content equals hash of target file content on disk → identical.
      - `copy` where source is symlink: target is symlink AND link target == `read_link(entry.source)` → identical.
      - If identical: if `overwrite_identical` flag set, update DB entry. Skip write. Return.
b. **Management Check** (if not identical): Determine if the target is unchanged since dotrift last wrote it.
      - Query DB for entry at target path. No entry → unmanaged.
      - If DB entry exists:
        - DB type is symlink: target symlink points to `DB.source_path` → managed.
        - DB type is copy with stored hash: target is regular file with on-disk hash matching `DB.hash` → managed.
        - DB type is copy without hash (NULL): `DB.source_path` is a symlink on disk, and target is symlink pointing to `read_link(DB.source_path)` → managed.
        - Otherwise → unmanaged.
  c. **Action:**
     - **Managed:** Proceed to step 4 (write) silently. Safe to overwrite — no external modification detected.
     - **Unmanaged:** Halt and prompt (`[s]kip / [o]verwrite / [q]uit`). Prompt loops until valid input.
       - **skip:** Do not touch the filesystem. Continue traversal.
       - **overwrite:** Delete the disk entity. Proceed to step 4.
       - **quit:** Immediately terminate the program.
4. **Write:**
    - `symlink`: unlink target (if exists) → `symlink(entry.source, target)`.
    - `copy`: if `entry.source` is a symlink on disk, create a symlink at target pointing to `read_link(entry.source)`. Otherwise, `fs::copy(source, target)`. After write: if `mode` set AND target is regular file → `chmod`.
5. **DB Sync:** Insert/update DB entry after every successful write. Fields:
   - `target_path`: absolute target path.
   - `deploy_type`: `symlink` or `copy` (from rule).
   - `source_path`: absolute source path (from portal entry).
   - `hash`: hex digest of source file content if source is regular file (same hash compared during identical and managed checks), NULL if source is symlink or deploy type is `symlink`.

---

## `unapply` Command

Reverses the `apply` process, removing managed files from the target. Config must be loadable (same as `apply`) for target resolution and path validation.

**Usage:** `dotrift unapply [OPTIONS]`

**Options:**
* `--dry-run`: Print planned operations without touching the filesystem or database.
* `--prune-empty-dirs`: Recursively delete orphaned empty directories.

### Execution Pipeline

1. **`--dry-run` Behavior (if active):**
   * Iterate all DB entries.
   * Print `[REMOVE] <path>`: File is managed and exists on disk (would be deleted from disk and DB).
   * **Note:** Do not simulate recursive empty directory deletion for `--prune-empty-dirs`; only show the file deletions that would initiate the process.

2. **Execution (if not `--dry-run`):** Iterate all DB entries.
   * **Managed File:** Delete from disk. Delete from DB. ("Managed" determined by the same logic as `apply` step 3b.)
   * **Unmanaged/Missing File:** Do not touch disk. Delete from DB.

3. **Prune Phase:** If `--prune-empty-dirs` is active, after all file removals, recursively delete empty directories starting from leaf directories upward. No upper boundary — may prune the target directory itself or its ancestors.

---

### `add` Command
**Usage:** `dotrift add [OPTIONS] <PATH> <DESTINATION>`

**Arguments:**
* `<PATH>`: Absolute path to existing file, directory, or symlink on disk. Any file type accepted; move/copy preserves the type.
* `<DESTINATION>`: Path in source directory. Relative to source-dir if not absolute. Error if normalized path escapes the source directory.

**Options:**
* `-c, --copy`: Copy instead of moving the file or directory to the destination.
* `-f, --force`: Remove all obstructions (files, directories — recursively including non-empty directories) blocking the move/copy.
* `-e, --editor <WHEN>`: Whether to open `dotrift.toml` with your editor. Values: `always`, `never`.

### Execution Pipeline
1. **Resolve Destination:** If relative, join `source-dir` + `<DESTINATION>`. Normalize path.
2. **Validate:** After path normalization, error if resolved destination escapes the source directory.
3. **Collision Check:** Error if resolved destination path already exists on disk (unless `--force`).
4. **Editor Decision:**
   - If `--editor never`: skip editor.
   - If `--editor always`: open editor. Error if no editor found.
   - If auto (no flag): open editor only if destination does not match any existing portal entry. A match occurs when: a literal key equals the destination path, a literal key is an ancestor of the destination path, or a glob key matches the destination path entirely. Error if no editor found.
   - Editor command resolved from global config `editor-command`, then `$VISUAL`, then `$EDITOR`.
5. **Clone File:** Create destination parent dirs. If `--copy`, recursively copy source to destination (dirs/symlinks preserved). Else, move source to destination. If the move crosses filesystem boundaries, error (do not fall back to copy+delete).

---

## `diff` Command

*To be implemented.*

---

## `status` Command

Reports the management status of the target filesystem.

**Usage:** `dotrift status <SUBCOMMAND> [OPTIONS]`

**Subcommands:**
* `list [file]`: List all managed files, or check a specific file.
* `clear [file]`: Clear status for a specific file, or all files if omitted.

### Execution Pipeline

**`list`:**
1. **Determine Scope:** Specific file or entire DB.
2. **Evaluate Status:** Check if file exists on disk and matches DB entry. Uses the same management check logic as `apply` step 3b (managed = on-disk state matches DB-recorded state).
3. **Output:**
   * `[MANAGED] target -> source (type)` *(In DB and on-disk state matches DB record)*
   * `[UNMANAGED] target` *(Not in DB, or on-disk state differs from DB record. Source path omitted for unmanaged entries.)*
  * If no file specified, prints all entries as `target -> source (type)`.

**`clear`:**
1. **Determine Scope:** Specific file or entire DB.
2. **Delete:** Remove entry from DB only. Files on disk are not touched. Remove all entries if no file specified.
