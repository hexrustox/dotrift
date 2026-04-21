# Dotrift Specification

This document defines the CLI structure, global behaviors, database schema, configuration format, and specifications for all `dotrift` subcommands. The manager evaluates configurations defined in `dotrift.toml`.

---

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

**Pruning:**
When `--prune-empty-dirs` is active, empty directories are recursively deleted from leaf upward with no upper boundary — the target directory and its ancestors may be pruned.

**Collision Prompt:**
When an obstruction is encountered during filesystem operations, the user is prompted with `[s]kip / [o]verwrite / [q]uit`. If stdin is not a TTY, defaults to `skip`. Unless otherwise noted:
- **skip:** Skip the operation, continue traversal.
- **overwrite:** Remove the obstruction and proceed.
- **quit:** Halt the program.

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

If `editor-command` is omitted, the `add` command falls back to `$VISUAL`, then `$EDITOR`.

---

## Database Schema

The local database tracks managed files. It is the single source of truth for resolving states in `diff`, `status`, `apply`, and `unapply`.

**Location:** `$XDG_STATE_HOME/dotrift/db.sqlite` (fallback: `$XDG_DATA_HOME/dotrift.sqlite`). Parent directories are created if they do not exist.

**Format:** SQLite. Single-instance assumption: only one `dotrift` process should access the database at a time. No locking or concurrency guarantees.

**Schema:**
```sql
CREATE TABLE managed_files (
    target_path TEXT PRIMARY KEY,
    deploy_type TEXT NOT NULL,
    source_path TEXT NOT NULL,
    hash TEXT,
    symlink_target TEXT
);
```

**Columns:**
* `target_path`: Absolute path of the managed file (primary key).
* `deploy_type`: Enum (`symlink` | `copy`).
* `source_path`: Absolute path in `source-dir`. Used to read content for copies, or verify link targets for symlinks.
* `hash`: Hex digest using `xxHash64` of **source** file content at last apply. NULL for symlinks. Used to detect external modifications to the target (managed check compares target-on-disk hash against DB hash).
* `symlink_target`: The symlink destination stored at deploy time (i.e., `read_link(source_path)`). Present when deploy type is `copy` and source is a symlink. NULL otherwise. Decouples managed check from current source filesystem state.

---

## Configuration (`dotrift.toml`)

Defines the file mapping, filtering, and deployment rules.

```toml
# Optional root-level keys
target-directory = "/absolute/path"
ignore = ["pattern1", "pattern2"]

[portal]
"source_pattern" = "target_path"

[rule]
"target_pattern" = { type = "symlink", mode = "600" }
```

### Root Keys

#### `target-directory`
* **Type:** String (absolute path)
* **Default:** `$HOME` (handled in code, not via TOML env var expansion)
* **Description**: The root directory where files will be mapped to. If omitted, the application defaults to the user's home directory. Can be overridden via CLI argument.

#### `ignore`
* **Type:** Array of strings
* **Default:** `[]`
* **Description:** A list of patterns defining deployment targets to exclude. This is useful for temporarily disabling a mapping or resolving ambiguities when one source file is mapped to multiple locations.
* **Syntax:** Follows exact **Gitignore-style semantics** (including `!` for negation and trailing `/` for directory-only matching). Order matters — `!` patterns re-include previously excluded paths.
* **Matching Context:** Matched against the **resolved target path**, relative to `target-directory`.

### `[portal]` (Mapping Logic)

Defines the routing of files from the source to the target.

* **Keys:** Bash-like glob patterns matched against the source file path. 
* **Values:** The destination path relative to `target-dir`.
* **Mapping Types:**
  * **Literal Keys:** If the key is a literal path (no wildcards), it maps exactly one source file or directory to the exact target path. (e.g., `"config.ini" = ".config/app/config.ini"`).
  * **Glob Keys:** If the key contains a wildcard, the value must be a directory path. The stripped remainder of the source path is appended to this directory value to form the final target path.

If a file matches multiple keys, they will all be applied if not colliding at `target-dir`.

#### Path Stripping Rule (For Wildcards)

When a `[portal]` key contains a wildcard, the target path is calculated by stripping a prefix from the matched source path.  

The **stripping prefix** is the portion of the key *up to but not including* the first path component that contains any wildcard character (`*`, `?`, or `[]`). If the very first path component contains a wildcard, the stripping prefix is empty.

This prefix is removed from the beginning of the source path that matched the glob. The remainder is then appended to the value specified in the `[portal]` table.

**Literal keys** (those containing no wildcards) are not subject to this rule — they map the source path exactly to the target path given.

#### Examples

* **Literal File:** `"file1" = "file2"`  
  → Maps the source file `file1` directly to the target path `file2` (no stripping occurs).

* **Literal Directory:** `"dir1" = "dir2"`  
  → Maps the source directory `dir1` recursively (including all contents and subdirs) to the target path `dir2` (no stripping occurs).

* **Glob (Subdirectory):** `"src/**/*.rs" = "dist"`  
  → Path components of key: `src`, `**/*.rs`.  
  → First component containing a wildcard is the second one.  
  → Stripping prefix = `"src/"`.  
  → Source file `src/foo/bar.rs` → after stripping → `foo/bar.rs` → appended to `"dist"` → final target = `dist/foo/bar.rs`.

* **Glob (Root):** `"**" = "."`  
  → Path components of key: `**`.  
  → First component contains a wildcard.  
  → Stripping prefix = `""` (empty).  
  → Source file `foo/bar.rs` → after stripping → `foo/bar.rs` → appended to `"."` → final target = `./foo/bar.rs` (maps entire source root to target root).

* **Glob (Filename wildcard only):** `"conf*.ini" = "settings"`  
  → Path components of key: `conf*.ini`.  
  → First (and only) component contains a wildcard.  
  → Stripping prefix = `""` (empty).  
  → Source file `config.ini` → after stripping → `config.ini` (full filename is kept) → final target = `settings/config.ini`.

#### Recursion & Multi-Match
* Literal directories and globs like `"dir/**"` recurse into directories, mapping contents.
* Empty directories in source are ignored (no target mapping).
* A source file or directory can match multiple portals, mapping to multiple targets if no target path collision.

### `[rule]` (Deployment Logic)

Defines the deployment method (`type`) and file permissions (`mode`), directory rule is not supported.

* **Keys:** Bash-like glob patterns matched against the **resolved target path**, relative to `target-dir`.
* **Values:** Object containing `type` and/or `mode`.
* **Scope:** **File-only.** No implicit recursive directory matching.
* **Precedence:** Evaluated in exact configuration order (guaranteed via `indexmap`). The properties are shallow-merged. Last rule wins on conflict.

* **Empty Tables:** Empty `[portal]` or `[rule]` tables are valid and treated as no-ops.

#### Properties
* `type` (String): `"symlink"` or `"copy"`. Defaults to `"symlink"` if no rule matches.
* `mode` (String): File permissions represented as an octal string of digits only (e.g., `"600"`, `"0600"`). No `0o` prefix. Must be a valid octal value in the range `000`–`777` (error otherwise). Defaults to none (no explicit modification) if omitted.

### Globbing & Path Normalization

* **`[portal]` Syntax:** Supports standard **bash-like globbing** (`*`, `**`, `?`, `[]`). Uses `glob` crate. Brace expansion (`{}`) is not supported.
* **`ignore` Syntax:** Supports **Gitignore-style semantics**.
* **Prefix Normalization:** The `./` prefix is purely cosmetic. `"a" = "b"` and `"./a" = "./b"` are identical. Absolute paths in `[portal]` keys (e.g., `"/etc/foo"`) are normalized as relative to the source directory (e.g., `"./etc/foo"`).
* **Path Traversal Clamping:** `../` components that would escape `source-dir` or `target-directory` are clamped to the respective root. Paths cannot reference files outside the source or target directory.
* **Symlinks in Source:** Symbolic links encountered in the source directory are treated as regular files. The manager does not follow them to resolve their targets during discovery; the symlink itself is deployed.
  * When `type = "copy"`: the symlink itself is copied — target becomes a symlink pointing to the same destination as the source symlink (e.g., source→`/a/b`, target→`/a/b`).
  * When `type = "symlink"`: target becomes a symlink pointing to the source symlink path (e.g., source→`/a/b`, target→source).

### Validation & Error Handling

#### Errors (Halts Execution)
* **Invalid Target Directory:** If `target-directory` is provided but is not a valid absolute path.
* **Source-Target Overlap:** Error if `source-dir` equals `target-dir` (prevents self-modification loops).
* **Target Inside Source:** Error if `target-dir` is inside `source-dir` (prevents self-modification).
* **Empty Patterns:** If a `[portal]` key or value is an empty string.
* **Target Collisions:** If two different source paths resolve to the exact same target path. Show all colliding source paths and the collision target. Halts program.

#### Warnings (Continues Execution)
* **Invalid Mode on Symlink:** When `[rule]` attempts to apply a `mode` to a `type = "symlink"`.

---

## `apply` Command

Evaluates `dotrift.toml` and applies the defined state to the target filesystem.

**Usage:** `dotrift apply [OPTIONS]`
**Options:**
* `--dry-run`: Print planned operations without touching the filesystem or database.
* `--clean-up`: Remove previously managed files no longer mapped in `dotrift.toml`.
* `--prune-empty-dirs`: Recursively delete orphaned empty directories (see Pruning). Requires `--clean-up`.

### Execution Pipeline

#### Phase 1: State Resolution
1. Parse `dotrift.toml`, resolve target dir, normalize paths (see Path Normalization).
2. **Portal Resolution:** Glob `[portal]` keys against source files/dirs. Calculate `target_path` via Stripping Rule. Filter using `ignore` list (see Configuration). Store in `HashMap<TargetPath, (SourcePath, DeployType, Option<Mode>)>`. A source file/dir can match multiple portals if no target collision.
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
2. **Managed:** Delete file, delete from DB. If `--prune-empty-dirs` is active, prune empty directories (see Pruning).
3. **Unmanaged/Missing:** Do not touch disk. Delete from DB.

#### Phase 3: Execution
Traverse Rose Tree top-down (Pre-order DFS).

**Directory Nodes:**
* `fs::create_dir_all`. If fails due to existing non-directory (file, symlink [even if pointing to a directory], socket, etc.) -> Collision prompt.
  * **skip:** Abort subtree.
  * **overwrite:** Delete file then creates dir, deletes DB entry to avoid stale state, continue children.
  * **quit:** Halt program.
* Other errors abort subtree.

**File Nodes:**
1. **Exists?** No → Proceed to step 4 (write).
2. **Exists?** Yes → A Directory:
   * Collision prompt.
     * **skip:** Do not touch the filesystem. Continue traversal.
     * **overwrite:** Delete the directory recursively, delete DB entries under the directory. Proceed to step 4.
     * **quit:** Immediately terminate the program.
3. **Exists?** Yes → File or symlink on disk:
   a. **Identical Check:** Determine if the target matches what dotrift would write.
      - `symlink`: target is symlink AND link target == `entry.source` → identical.
      - `copy` where source is regular file: target is regular file AND hash of source file content equals hash of target file content on disk → identical.
      - `copy` where source is symlink: target is symlink AND link target == `read_link(entry.source)` → identical.
      - If identical: if `overwrite_identical` flag (see Global Configuration) set, update DB entry. Skip write. Return.
   b. **Management Check** (if not identical): Determine if the target is unchanged since dotrift last wrote it.
      - Query DB for entry at target path. No entry → unmanaged.
      - If DB entry exists:
        - DB type is symlink: target symlink points to `DB.source_path` → managed.
- DB type is copy with stored hash: target is regular file with on-disk hash matching `DB.hash` → managed.
         - DB type is copy with `symlink_target`: target is a symlink pointing to `DB.symlink_target` → managed.
        - Otherwise → unmanaged.
   c. **Action:**
      - **Managed:** Proceed to step 4 (write) silently. Safe to overwrite — no external modification detected.
      - **Unmanaged:** Collision prompt.
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
   - `symlink_target`: `read_link(source_path)` if source is a symlink and deploy type is `copy`, NULL otherwise.

---

## `unapply` Command

Reverses the `apply` process, removing managed files from the target. Config must be loadable (same as `apply`) for target resolution and path validation.

**Usage:** `dotrift unapply [OPTIONS]`

**Options:**
* `--dry-run`: Print planned operations without touching the filesystem or database.
* `--prune-empty-dirs`: Recursively delete orphaned empty directories (see Pruning). Stands alone (unlike `apply` where it requires `--clean-up`) because `unapply` inherently removes all managed files.

### Execution Pipeline

1. **`--dry-run` Behavior (if active):**
   * Iterate all DB entries.
   * Print `[REMOVE] <path>`: File is managed and exists on disk (would be deleted from disk and DB).
   * **Note:** Do not simulate recursive empty directory deletion for `--prune-empty-dirs`; only show the file deletions that would initiate the process.

2. **Execution (if not `--dry-run`):** Iterate all DB entries.
   * **Managed File:** Delete from disk. Delete from DB. ("Managed" determined by the same logic as `apply` step 3b.)
   * **Unmanaged/Missing File:** Do not touch disk. Delete from DB.

3. **Prune Phase:** If `--prune-empty-dirs` is active, prune empty directories (see Pruning).

---

## `add` Command

**Usage:** `dotrift add [OPTIONS] <PATH> <DESTINATION>`

**Arguments:**
* `<PATH>`: Absolute path to existing file, directory, or symlink on disk. Any file type accepted; move/copy preserves the type.
* `<DESTINATION>`: Path in source directory. Relative to source-dir if not absolute.

**Options:**
* `-c, --copy`: Copy instead of moving the file or directory to the destination.
* `-f, --force`: Remove all obstructions (files, directories — recursively including non-empty directories) blocking the move/copy.
* `-e, --editor <WHEN>`: Whether to open `dotrift.toml` with your editor. Values: `always`, `never`.

### Execution Pipeline
1. **Resolve Destination:** If relative, join `source-dir` + `<DESTINATION>`. Normalize path (see Path Normalization).
2. **Validate:** Error if resolved destination escapes the source directory (see Path Traversal Clamping).
3. **Collision Check:** Error if resolved destination path already exists on disk (unless `--force`).
4. **Editor Decision:**
   - If `--editor never`: skip editor.
   - If `--editor always`: open editor. Error if no editor found.
   - If auto (no flag): open editor only if destination does not match any existing portal entry. A match occurs when: a literal key equals the destination path, a literal key is an ancestor of the destination path, or a glob key matches the destination path entirely. Error if no editor found.
   - Editor command resolved per Global Configuration.
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
