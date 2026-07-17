# `add`

**Usage:** `dotrift add [OPTIONS] <PATH> [DESTINATION]`

**Arguments:**

* `<PATH>`: Path to an existing file, directory, or symlink on disk. If relative, resolved against cwd.
* `[DESTINATION]`: Optional path in the source directory. When omitted, *re-import mode* is active (destination derived from DB; see Re-import Mode below).

**Options:**

* `-c, --copy`: Copy instead of moving. Implicit in re-import mode.
* `-f, --force`: Remove obstructions blocking the move/copy.
* `-e, --editor <WHEN>`: Values: `always`, `never`. Default: auto (open editor if conditions below are met).
* `-n, --no-modify`: Do not modify `dotrift.toml` (skip auto-add and collision annotations). Editor may still open for manual configuration.

---

## Definitions

* `dest_rel` — the resolved destination path relative to `source-dir`; this is the portal key used by auto-added entries.
* `computed_target` — derived from `<PATH>` by stripping the target directory prefix; if `<PATH>` is not under the target directory, the absolute path is used instead (this triggers the `# WARNING:` comment in step 6).

## Re-import Mode

<a id="re-import-mode"></a>

When `DESTINATION` is omitted, `add` is in *re-import mode* (term defined in
`CONTEXT.md`): the destination is derived from the existing database entry
for the target file. Used to pull an externally-edited file back into the
source tree. The DB must exist and contain an entry whose `target_path`
matches `<PATH>`, otherwise the command errors.

---

## Execution Pipeline

### Directory PATH

* **Standard mode:** Recursively copy/move directory contents. Apply pipeline to directory as unit. Portal analysis and editor decision work the same as for a single file: `dest_rel` is the directory's destination relative to `source-dir`. Auto-add produces a single literal directory portal entry.
* **Re-import mode:** Walk directory. For each file, apply pipeline individually. The editor opens once for the entire batch. Files not found in the database are skipped with a warning. Each file that needs one gets its own auto-added portal entry.

### Single file / symlink

1. **Normalize PATH:** Resolve to absolute path.
2. **Resolve Destination:**
   - *Standard mode* (DESTINATION provided): If relative, join `source-dir` + DESTINATION. If absolute, use as-is. Normalize. Error if escapes source-dir.
   - *Re-import mode* (DESTINATION omitted): Query DB for `target_path` == PATH. Error if the database does not exist or no entry is found. Set destination to `entry.source_path`.
3. **Collision Check:** Error if the destination path exists on disk — file, directory, or any other type (including an empty directory). If `--force`, remove the obstruction (recursively for directories). (This is *not* the collision prompt; see `spec/prompt.md`.)
4. **Clone:**
   - *Standard mode:* If `--copy`, copy. Else, move.
   - *Re-import mode:* Always copy.
   - Symlink handling during clone follows the `type = "copy"` Source Symlink Behavior (`spec/dotrift-toml.md#source-symlink-behavior`): the symlink itself is copied/moved, preserving its link target. It is never followed.
5. **Resolve Target Directory:** Determine target directory using full precedence (see `spec/core.md` Target Directory Precedence).
6. **Analyze Portal Mapping:** Check whether any existing portal key matches `dest_rel`, and whether the auto-added entry would collide with existing entries (see `spec/dotrift-toml.md` `[portal]`):
   - **Missing key:** No portal key matches `dest_rel` — a new entry is needed.
   - **Target collision:** Another portal entry (different key) maps some source file to the same target path as `dest_rel` would. This would cause `apply` to halt with a collision error (see `spec/dotrift-toml.md` Validation & Errors).
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
9. **Open Editor (if decision was yes):** Editor invocation uses the `editor-command` config (see `spec/global-config.md`).
   - If a temp file was prepared: open editor on the temp file. After exit:
     - **File saved:** copy temp to real config.
     - **File not saved:** discard temp (config unchanged).
   - If no temp file: open editor on the real `dotrift.toml` directly.
   - When the editor opens, `{file}` is the path to the file being edited (temp file or real `dotrift.toml`). `{row}` positions the cursor at the auto-added portal entry, or at the end of the `[portal]` section if no entry was auto-added.
   - If no editor is available (none of `editor-command`, `$VISUAL`, or `$EDITOR` are set), error and halt.