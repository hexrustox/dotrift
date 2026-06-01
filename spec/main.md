# Dotrift Specification

This document defines the CLI structure, global behaviors, database schema, configuration format, and specifications for all `dotrift` subcommands. The manager evaluates configurations defined in `dotrift.toml`, which is processed as a [template](#dotrift-template-syntax) before being parsed.

---

## Global CLI Structure

**Usage:** `dotrift [GLOBAL OPTIONS] <COMMAND> [COMMAND ARGS/OPTIONS]`

### Global Options
* `-s, --source <DIR>`: Path to the source directory containing `dotrift.toml` and dotfiles. Default: `~/.local/share/dotrift`.
* `-t, --target <DIR>`: Override the target directory.
* `-c, --config <FILE>`: Override the global config file path. Default: `$XDG_CONFIG_HOME/dotrift/config.toml`.
* `-v, --verbose`: Enable verbose logging.

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
When an obstruction is encountered during filesystem operations, the user is prompted with `[s]kip / [o]verwrite / [d]iff / [q]uit`. If stdin is not a TTY, defaults to `skip` is not offered. Unless otherwise noted:
- **skip:** Skip the operation, continue traversal.
- **overwrite:** Remove the obstruction and proceed.
- **diff:** Open the pager TUI (see `spec/pager.md`). After the pager exits, re-display the full collision prompt.
- **quit:** Halt the program.

### Global Configuration

Loaded from `$XDG_CONFIG_HOME/dotrift/config.toml`. This file is optional; missing files are treated as empty (all defaults apply). Invalid or malformed TOML is an error. Partial configs merge with defaults.

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
  * `args` (array of strings): Arguments passed to the command. Supports parameter expansion (see below).

If `editor-command` is omitted, the `add` command falls back to `$VISUAL`, then `$EDITOR`.

#### Parameter Expansion

The `args` array supports parameter expansion using `{param}` syntax.

| Parameter | Description |
|-----------|-------------|
| `{file}` | Absolute path to the file being opened |
| `{row}` | Line number (1-indexed) |
| `{col}` | Column number (1-indexed) |

**Rules:**
* `{param}` in args strings is replaced with the parameter value.
* All parameters are guaranteed to be set by the program.
* Unknown parameter: error.
* Literal braces: `{{` and `}}` produce `{` and `}`.
* No shell expansion — args passed directly to `Command::new()`.

**Example:**

```toml
[editor-command]
command = "vim"
args = ["-f", "{file}", "+{row}"]
```

Expands to: `vim -f /path/to/dotrift.toml +42`

---

## Database Tables

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
    symlink_target TEXT,
    mtime INTEGER
);
```

**Columns:**
* `target_path`: Absolute path of the managed file (primary key).
* `deploy_type`: Enum (`symlink` | `copy` | `tmpl`).
* `source_path`: Absolute path in `source-dir`. Used to read content for copies, or verify link targets for symlinks.
* `hash`: Hex digest using `xxHash64` of target file content at last apply. NULL for symlinks. Used to detect external modifications to the target (managed check compares target-on-disk hash against DB hash).
* `symlink_target`: `read_link(source_path)` if source is a symlink and deploy type is `copy`, NULL otherwise. Decouples managed check from current source filesystem state.
* `mtime`: Modification time of the target file at last apply, stored as milliseconds since Unix epoch. NULL for symlinks. When the on-disk mtime matches this value, the file is considered managed without computing the hash.

Tracks which template profiles are currently active. The activation timestamp determines variable precedence during template evaluation — last-activated (highest `activated_at`) wins on conflict.

```sql
CREATE TABLE IF NOT EXISTS active_profiles (
    activated_at INTEGER NOT NULL,
    name         TEXT NOT NULL UNIQUE
);
```

**Columns:**
* `activated_at`: Unix timestamp in milliseconds when the profile was activated. Last-activated (highest timestamp) wins on variable conflict.
* `name`: Profile name, matches a `[profile.<name>]` section in `dotrift_data.toml`.

---

## Configuration (`dotrift.toml`)

Defines the file mapping, filtering, and deployment rules. Before being parsed, `dotrift.toml` is evaluated as a template (see [Template Syntax](#dotrift-template-syntax)). The evaluated result must be valid TOML conforming to the structure below. Template context is resolved from `dotrift_data.toml` (see [Template Data](#template-data-dotrift_datatoml)).

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
* **Implicit Ignores:** `dotrift.toml` and `dotrift_data.toml` are implicitly excluded from deployment. They match no portal and are never written to the target directory.

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
* `type` (String): `"symlink"`, `"copy"`, or `"tmpl"`. Defaults to `"symlink"` if no rule matches.
  * **tmpl:** The source file is evaluated as a template (see [Template Syntax](#dotrift-template-syntax)) before being written to the target.
* `mode` (String): File permissions represented as an octal string of digits only (e.g., `"600"`, `"0600"`). No `0o` prefix. Must be a valid octal value in the range `000`–`777` (error otherwise). Defaults to none (no explicit modification) if omitted.

### Globbing & Path Normalization

* **`[portal]` Syntax:** Supports standard **bash-like globbing** (`*`, `**`, `?`, `[]`). Uses `glob` crate. Brace expansion (`{}`) is not supported.
* **`ignore` Syntax:** Supports **Gitignore-style semantics**.
* **Prefix Normalization:** The `./` prefix is purely cosmetic. `"a" = "b"` and `"./a" = "./b"` are identical. Absolute paths in `[portal]` keys (e.g., `"/etc/foo"`) are normalized as relative to the source directory (e.g., `"./etc/foo"`).
* **Path Normalization Clamping:** Portal keys are normalized as relative paths. `../` components in a relative path resolve against the path's own root — leading `..` components that would "escape" are preserved but cannot reference above the root of a relative path. When joined with `source-dir` or `target-directory`, the resulting absolute path stays within the source/target tree. The same normalization applies to target-side paths. No explicit clamping is needed; path normalization provides this guarantee.
* **Symlinks in Source:** Symbolic links encountered in the source directory are treated as regular files. The manager does not follow them to resolve their targets during discovery; the symlink itself is deployed.
  * When `type = "copy"`: the symlink itself is copied — target becomes a symlink pointing to the same destination as the source symlink (e.g., source→`/a/b`, target→`/a/b`).
  * When `type = "symlink"`: target becomes a symlink pointing to the source symlink path (e.g., source→`/a/b`, target→source).
  * When `type = "tmpl"`: follow the symlink chain to its ultimate resolution, read the resolved file's content, parse as template, evaluate, write result to target. Source must resolve to a regular file (error otherwise).

### Validation & Error Handling

#### Errors (Halts Execution)
* **Invalid Target Directory:** If `target-directory` is provided but is not a valid absolute path.
* **Source-Target Overlap:** Error if `source-dir` equals `target-dir` (prevents self-modification loops).
* **Target Inside Source:** Error if `target-dir` is inside `source-dir` (prevents self-modification).
* **Target Collisions:** If two different source paths resolve to the exact same target path. Show all colliding source paths and the collision target. Halts program.

---

## Template Data (`dotrift_data.toml`)

Located in the source directory, next to `dotrift.toml`. Optional. When present, variables feed into both `dotrift.toml` evaluation and `tmpl`-type source file rendering.

### Format

```toml
[variable]
key = "value"

[profile.<name>]
key = "override"
```

### Sections

**`[variable]`:** Key-value pairs forming the base template context. Values can be any valid TOML type: string, integer, boolean, array, or inline table. These map to the template value types String, Int, Bool, List, and Map respectively.

**`[profile.<name>]`:** Named profiles with additional or overriding variables. Same value types as `[variable]`. Activated via `dotrift profile activate <name>` (see [`profile` Command](#profile-command)). Multiple profiles can be active simultaneously.

### Resolution

1. `[variable]` is the base layer.
2. Active profiles (from DB, in activation order) overlay on top.
3. Last-activated profile (highest `activated_at`) wins on conflict.
4. Profiles active in DB but missing from `dotrift_data.toml` are silently ignored.

### Errors

Parse errors are fatal. Missing file is treated as empty (no variables defined).

## Template Syntax

See `spec/templater.md` for the full template syntax specification.

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
1. **Template Evaluation:** Load `dotrift_data.toml` from source dir (if present), query DB for active profiles, resolve template context. Read `dotrift.toml`, evaluate as template, then parse the result as TOML config. Template errors (parse or render) are fatal.
2. **Portal Resolution:** Glob `[portal]` keys against source files/dirs. Calculate `target_path` via Stripping Rule. Filter using `ignore` list (see Configuration). Store in `HashMap<TargetPath, (SourcePath, DeployType, Option<Mode>)>`. A source file/dir can match multiple portals if no target collision.
   * Literal directory keys expand to one entry per contained file (not one entry for the directory itself).
   * *Error:* Target path collision. Show all colliding source paths and the collision target. Halt program.
3. **Rule Application:** Apply `[rule]` in order, shallow-merging properties to determine final `type` and `mode`. Last rule wins on conflict.

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

1.  **Clean-up Plan (if `--clean-up` is also passed):** The dry run simulates the clean-up phase first. It iterates the database and prints `[REMOVE] <path>` for any entry not found in the Phase 1 portal entries HashMap.

2.  **Deployment Plan:** The structured tree of files to be created is printed.
    *   `[CREATE] /path/to/file (symlink)`
    *   `[CREATE] /path/to/file (file)`
    *   `[CREATE] /path/to/file (tmpl)`

**Note:** The dry run does not simulate recursive deletion of empty directories (`--prune-empty-dirs`).

#### `--clean-up` Behavior
If active, execute after Phase 1, before Phase 2. If `--dry-run` is also active, print planned removals without modifying FS or DB. Otherwise:
Iterate DB. If a path is NOT in the Phase 1 portal entries HashMap:
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
  * **diff:** Open pager in single-pane mode, displaying the obstructing file's content with a header noting the directory that dotrift needs to create.
* Other errors abort subtree.

**File Nodes:**
1. **Exists?** No → Proceed to step 4 (write).
2. **Exists?** Yes → A Directory:
   * Collision prompt.
     * **skip:** Do not touch the filesystem. Continue traversal.
     * **overwrite:** Delete the directory recursively, delete DB entries under the directory. Proceed to step 4.
     * **quit:** Immediately terminate the program.
     * **diff:** Open pager in explorer mode. Left pane: source file content. Right pane: file browser at the target directory.
3. **Exists?** Yes → File or symlink on disk:
   a. **Identical Check:** Determine if the target matches what dotrift would write.
       - `symlink`: target is symlink AND link target == `entry.source` → identical.
       - `copy` where source is regular file: target is regular file AND hash of source file content equals hash of target file content on disk → identical.
       - `copy` where source is symlink: target is symlink AND link target == `read_link(entry.source)` → identical.
       - `tmpl`: never identical. Always proceed to management check.
       - If identical: if `overwrite_identical` flag (see Global Configuration) set, update DB entry. Skip write. Return.
   b. **Management Check** (if not identical): Determine if the target is unchanged since dotrift last wrote it.
        - Query DB for entry at target path. No entry → unmanaged.
        - If DB entry exists:
          - DB type is symlink: target symlink points to `DB.source_path` → managed.
          - DB type is copy or tmpl with stored hash: target is a regular file. If `DB.mtime` is non-NULL and matches the on-disk mtime → managed (skip hash). Otherwise, fall back to hash: on-disk hash matches `DB.hash` → managed, else → unmanaged.
          - DB type is copy with `symlink_target`: target is a symlink pointing to `DB.symlink_target` → managed.
          - Otherwise → unmanaged.
   c. **Action:**
      - **Managed:** Proceed to step 4 (write) silently. Safe to overwrite — no external modification detected.
      - **Unmanaged:** Collision prompt.
        - **skip:** Do not touch the filesystem. Continue traversal.
        - **overwrite:** Delete the disk entity. Proceed to step 4.
        - **quit:** Immediately terminate the program.
        - **diff:** Open pager in side-by-side mode. Left pane: source file. Right pane: target file on disk.
4. **Write:**
    - `symlink`: unlink target (if exists) → `symlink(entry.source, target)`.
    - `copy`: if `entry.source` is a symlink on disk, create a symlink at target pointing to `read_link(entry.source)`. Otherwise, `fs::copy(source, target)`. After write: if `mode` set AND target is regular file → `chmod`.
    - `tmpl`: resolve template context from `dotrift_data.toml` (base + active profiles). If source is a symlink, resolve it first. Parse the source file as a template, evaluate with context, write rendered output to target via streaming writer. After write: if `mode` set AND target is regular file → `chmod`.
5. **DB Sync:** Insert/update DB entry after every successful write. Fields:
    - `target_path`: absolute target path.
    - `deploy_type`: `symlink`, `copy`, or `tmpl` (from rule).
    - `source_path`: absolute source path (from portal entry).
    - `hash`: hex digest of target file content if source resolves to a regular file (same hash compared during identical and managed checks), NULL if source is a symlink or deploy type is `symlink`. For `tmpl`, hash is computed on the **rendered** target file.
    - `symlink_target`: `read_link(source_path)` if source is a symlink and deploy type is `copy`, NULL otherwise.
    - `mtime`: modification time of the target file after write, read via `symlink_metadata().modified()`, stored as milliseconds since Unix epoch. NULL for symlinks.

---

## `unapply` Command

Reverses the `apply` process, removing managed files from the target. Config must be loadable (same as `apply`) for target resolution and path validation.

**Usage:** `dotrift unapply [OPTIONS]`

**Options:**
* `--dry-run`: Print planned operations without touching the filesystem or database.
* `--prune-empty-dirs`: Recursively delete orphaned empty directories (see Pruning). Stands alone (unlike `apply` where it requires `--clean-up`) because `unapply` inherently removes all managed files.

### Execution Pipeline

0. **Load Config:** Same as `apply` Phase 1 Step 1 — evaluate `dotrift.toml` as template, parse result. Used for target directory resolution and to determine which portal entries exist under the current config. Only files currently matched by configured portals are eligible for unapply.
1. **`--dry-run` Behavior (if active):**
   * Iterate all DB entries.
   * Print `[REMOVE] <path>`: File is managed and exists on disk (would be deleted from disk and DB).
   * **Note:** Do not simulate recursive empty directory deletion for `--prune-empty-dirs`; only show the file deletions that would initiate the process.

2. **Execution (if not `--dry-run`):** Iterate all DB entries. Skip any entry whose target path is not matched by a currently configured portal.
    * **Managed File:** Delete from disk. Delete from DB. ("Managed" determined by the same logic as `apply` step 3b.)
    * **Unmanaged/Missing File:** Do not touch disk. Delete from DB.

3. **Prune Phase:** If `--prune-empty-dirs` is active, prune empty directories (see Pruning).

---

## `add` Command

**Usage:** `dotrift add [OPTIONS] <PATH> [DESTINATION]`

**Arguments:**
* `<PATH>`: Path to existing file, directory, or symlink on disk. If relative, resolved against cwd.
* `[DESTINATION]`: Optional path in source directory. When omitted, re-import mode (destination derived from DB).

**Options:**
* `-c, --copy`: Copy instead of moving. Implicit in re-import mode.
* `-f, --force`: Remove obstructions blocking the move/copy.
* `-e, --editor <WHEN>`: Values: `always`, `never`. Default: auto (open editor if conditions below are met).
* `-n, --no-modify`: Do not modify `dotrift.toml` (skip auto-add and collision annotations). Editor may still open for manual configuration.

### Execution Pipeline

If PATH is a directory:
- *Standard mode:* Recursively copy/move directory contents. Apply pipeline to directory as unit. Portal analysis and editor decision work the same as for a single file: `dest_rel` is the directory's destination relative to `source-dir`. Auto-add produces a single literal directory portal entry.
- *Re-import mode:* Walk directory. For each file, apply pipeline individually. The editor opens once for the entire batch. Files not found in the database are skipped with a warning. Each file that needs one gets its own auto-added portal entry.

Otherwise (single file/symlink):

**Definitions:** `dest_rel` is the resolved destination path relative to `source-dir` (the portal key in auto-added entries). `computed_target` is derived from `<PATH>` by stripping the target directory prefix; if `<PATH>` is not under the target directory, the absolute path is used instead (this triggers the `# WARNING:` comment in step 6).

1. **Normalize PATH:** Resolve to absolute path.
2. **Resolve Destination:**
   - *Standard mode* (DESTINATION provided): If relative, join `source-dir` + DESTINATION. If absolute, use as-is. Normalize. Error if escapes source-dir.
   - *Re-import mode* (DESTINATION omitted): Query DB for `target_path` == PATH. Error if the database does not exist or no entry is found. Set destination to `entry.source_path`.
3. **Collision Check:** Error if the destination path exists on disk — file, directory, or any other type (including an empty directory). If `--force`, remove the obstruction (recursively for directories).
4. **Clone:**
   - *Standard mode:* If `--copy`, copy. Else, move.
   - *Re-import mode:* Always copy.
   - Symlink handling during clone follows the `type = "copy"` behavior (see Symlinks in Source): the symlink itself is copied/moved, preserving its link target. It is never followed.
5. **Resolve Target Directory:** Determine target directory using full precedence: CLI `-t` > config `target-directory` > `$HOME`.
6. **Analyze Portal Mapping:** Check whether any existing portal key matches `dest_rel`, and whether the auto-added entry would collide with existing entries:
   - **Missing key:** No portal key matches `dest_rel` — a new entry is needed.
   - **Target collision:** Another portal entry (different key) maps some source file to the same target path as `dest_rel` would. This would cause `apply` to halt with a collision error.
7. **Editor Decision** (based on step 4 — no config modifications yet):
   - `--editor never`: skip (also suppresses auto-add and annotations).
   - `--editor always`: open editor.
   - Auto (default): open editor if **Missing key** or **Target collision** is detected.
8. **Prepare Config Changes:** Performed only when the editor will open (step 5) and `--no-modify` is not set. Apply changes based on step 4:
   - **Missing key:** Append `"dest_rel" = "computed_target"` to the end of `[portal]`. Create `dotrift.toml` (with a `[portal]` section) if the file does not exist.
     - **Warning:** If `computed_target` is outside the target directory, prepend a `# WARNING:` comment above the new entry:
     ```toml
     # WARNING: <computed_target> is outside target directory <target_dir>
     "dest_rel" = "computed_target"
     ```
   - **Target collision:** For each unique target path with collisions, list the other colliding portal keys in a `# CONFLICT` comment placed directly above every portal entry (including any newly auto-added entry) that resolves to that target:
   ```toml
   [portal]
   # CONFLICT b, c
   "a" = "a"
   # CONFLICT a, c
   "b" = "a"
   # CONFLICT a, b
   "c" = "a"
   # CONFLICT y
   "x" = "b"
   # CONFLICT x
   "y" = "b"
   ```
   Write the modified content to a **temporary file**. If no changes are needed (no missing key and no collision), skip this step — the editor opens on the real config directly.
9. **Open Editor (if decision was yes):**
   - If a temp file was prepared: open editor on the temp file. After exit:
     - **File saved:** copy temp to real config.
     - **File not saved:** discard temp (config unchanged).
   - If no temp file: open editor on the real `dotrift.toml` directly.
   When the editor opens, `{file}` is the path to the file being edited (temp file or real `dotrift.toml`). `{row}` positions the cursor at the auto-added portal entry, or at the end of the `[portal]` section if no entry was auto-added.
   If no editor is available (none of `editor-command`, `$VISUAL`, or `$EDITOR` are set), error and halt.

---

## `diff` Command

Shows a side-by-side diff between a managed target file and its corresponding source file in the dotrift pager TUI.

**Usage:** `dotrift diff <PATH>`

**Arguments:**
* `<PATH>`: Path to a managed file to diff. If relative, resolved against cwd.

### Execution Pipeline

1. **Normalize PATH:** Resolve to absolute path.
2. **Open DB:** Initialize the database at the standard location (`$XDG_STATE_HOME/dotrift/db.sqlite` or fallback).
3. **Look Up PATH:** Query DB for `target_path` == `<PATH>`.
   * *Error:* No entry found — `<PATH>` is not managed by dotrift.
4. **Validate Source:** Check `entry.source_path` exists on disk via `symlink_metadata`. A broken symlink in source is treated as missing.
   * *Error:* Source file `<source_path>` not found.
5. **Validate Target:** Check `<PATH>` exists on disk via `symlink_metadata`. A broken symlink at the target is treated as missing.
   * *Error:* Target file `<PATH>` not found.
6. **Open Pager:** Open the pager TUI in side-by-side diff mode (see `spec/pager.md` — Diff Mode).
   * **Left pane:** Target file (the managed file on disk).
   * **Right pane:** Source file (the dotfile in the source directory).

---

## `init` Command

Initializes the source directory with a default `dotrift.toml`.

**Usage:** `dotrift init`

### Execution Pipeline

1. **Resolve Source Directory:** Use the source directory determined by global options.
2. **Check Existing Config:** Error if `dotrift.toml` already exists in the source directory.
3. **Create Config:** Write a default `dotrift.toml` to the source directory, creating parent directories as needed.

---

## `profile` Command

Manages template profiles.

**Usage:** `dotrift profile <SUBCOMMAND> [ARGS]`

**Subcommands:**
- `list`: Show all profiles from `dotrift_data.toml`, mark active ones.
- `activate <name>`: Activate a profile. Re-activating an already-active profile updates its timestamp to now (moves it to last in precedence).
- `deactivate <name>`: Deactivate a profile. Error if not active.
- `show`: Print the resolved variable context as a two-column key-value table.

### Execution Pipeline

**`list`:**
1. Parse `dotrift_data.toml`. Error if it is missing or has no `[profile]` entries.
2. Query DB for active profiles.
3. Print each profile name. Active ones annotated with `(active)`.

**`activate <name>`:**
1. Parse `dotrift_data.toml`.
2. Error if `<name>` is not defined in `[profile]`.
3. `INSERT OR REPLACE` into `active_profiles` (REPLACE updates `activated_at`).

**`deactivate <name>`:**
1. Delete from `active_profiles` where `name` = `<name>`. Error if profile is not active.

**`show`:**
1. Parse `dotrift_data.toml`.
2. Query active profiles in activation order.
3. Merge variables: `[variable]` base, then each profile in activation order (last wins).
4. Print as a two-column table (`key` | `value`). If no variables and no active profiles, print nothing.

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

---

## `templater` Command

Evaluates a dotrift template standalone and writes the rendered output to stdout or a file.

**Usage:** `dotrift templater [OPTIONS]`

**Template Source** (exactly one required):
* `-s, --string <TEMPLATE>`: Inline template string to evaluate.
* `-f, --file <PATH>`: Path to a template file on disk.

**Options:**
* `-o, --output <PATH>`: Write rendered output to the specified file instead of stdout. Parent directories are created if they do not exist.
* `-v, --var <KEY=VALUE>`: Set a template variable. Overrides `dotrift_data.toml` and active profiles. Repeatable. Value is parsed as a TOML literal (string, integer, boolean, array, or inline table). Parse errors are fatal.
* `--no-data`: Do not load `dotrift_data.toml` or active profiles. Only `--var` variables are available.
* `--data-path <PATH>`: Explicit path to `dotrift_data.toml`. When omitted, resolves from the source directory (respects `-s, --source`).

### Errors

* **Missing template source:** Error if neither `--string` nor `--file` is provided.
* **Ambiguous template source:** Error if both `--string` and `--file` are provided.
* **Input-output conflict:** Error if `--file` and `--output` resolve to the same path.
* **Mutually exclusive flags:** Error if both `--no-data` and `--data-path` are provided.
* **Template errors:** Parse and render errors are fatal, reported with source annotations.
* **`--var` parse errors:** Fatal.
* **DB errors:** Fatal.

### Execution Pipeline

1. **Resolve template source:** If `--string`, use the inline string directly. If `--file`, read the file from disk.
   *Error:* If `--output` resolves to the same path as `--file` (via `canonicalize`), bail before any I/O.

2. **Resolve variables** (unless `--no-data`):
   - Load `dotrift_data.toml` from `--data-path` if provided, or from the source directory. Missing file is treated as empty (no variables from file).
   - Query the database for active profiles. DB errors are fatal.
   - Merge in order: `[variable]` → active profiles (by activation order).

3. **Apply `--var` overrides:** Each `KEY=VALUE` argument sets a variable, parsed as a TOML literal. Parse errors are fatal. Overwrites any conflicting key from step 2.

4. **Evaluate template:** Evaluate the template with the resolved variable context and the same built-in functions available to `apply`. Template syntax follows `spec/templater.md`.

5. **Output:** Write rendered content to stdout, or to the file specified by `--output` (creating parent directories as needed).
