# Dotrift Configuration Specification

This document defines the formal specification for the dotfile manager's `dotrift.toml` file.

## Overview

The manager maps files from a source directory (where dotfiles are stored) to a target directory (e.g., `$HOME`). It determines *where* files go (`[portal]`), filters unwanted files (`ignore`), and defines *how* they are deployed and permissioned (`[rule]`).

## Configuration Structure

```toml
# Optional root-level keys
target-dir = "/absolute/path"
ignore = ["pattern1", "pattern2"]

[portal]
"source_pattern" = "target_path"

[rule]
"target_pattern" = { type = "symlink", mode = "600" }
```

---

## Root Keys

### `target-dir`
* **Type:** String (absolute path)
* **Default:** `$HOME` (handled in code, not via TOML env var expansion)
* **Description**: The root directory where files will be mapped to. If omitted, the application defaults to the user's home directory.

### `ignore`
* **Type:** Array of strings
* **Default:** `[]`
* **Description:** A list of patterns defining deployment targets to exclude. This is useful for temporarily disabling a mapping or resolving ambiguities when one source file is mapped to multiple locations.
* **Syntax:** Follows exact **Gitignore-style semantics** (including `!` for negation and trailing `/` for directory-only matching).
* **Matching Context:** Matched against the **resolved target path**, relative to `target-dir`.

---

## `[portal]` (Mapping Logic)

Defines the routing of files from the source to the target.

* **Keys:** Bash-like glob patterns matched against the source file path. 
* **Values:** The destination path relative to `target-dir`.
* **Mapping Types:**
  * **Literal Keys:** If the key is a literal path (no wildcards), it maps exactly one source file or directory to the exact target path. (e.g., `"config.ini" = ".config/app/config.ini"`).
  * **Glob Keys:** If the key contains a wildcard, the value must be a directory path. The stripped remainder of the source path is appended to this directory value to form the final target path.

If a file matches multiple keys, they will all be applied if not colliding at `target-dir`.

### Path Stripping Rule (For Wildcards)

When a `[portal]` key contains a wildcard, the target path is calculated by stripping a prefix from the matched source path.  

The **stripping prefix** is the portion of the key *up to but not including* the first path component that contains any wildcard character (`*`, `?`, `[]`, or `{}`). If the very first path component contains a wildcard, the stripping prefix is empty.

This prefix is removed from the beginning of the source path that matched the glob. The remainder is then appended to the value specified in the `[portal]` table.

**Literal keys** (those containing no wildcards) are not subject to this rule — they map the source path exactly to the target path given.

### Examples

* **Literal File:** `"file1" = "file2"`  
  → Maps the source file `file1` directly to the target path `file2` (no stripping occurs).

* **Literal Directory:** `"dir1" = "dir2"`  
  → Maps the source directory `dir1` (and all its contents) directly to the target path `dir2` (no stripping occurs).

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

---

## `[rule]` (Deployment Logic)

Defines the deployment method (`type`) and file permissions (`mode`), directory rule is not supported.

* **Keys:** Bash-like glob patterns matched against the **resolved target path**, relative to `target-dir`.
* **Values:** Object containing `type` and/or `mode`.
* **Scope:** **File-only.** No implicit recursive directory matching.
* **Precedence:** Evaluated in exact configuration order (guaranteed via `indexmap`). The properties are shallow-merged.

### Properties
* `type` (String): `"symlink"` or `"copy"`. Defaults to `"symlink"` if no rule matches.
* `mode` (String): File permissions represented as an octal string (e.g., `"600"`). Defaults to none (no explicit modification) if omitted.

---

## Globbing & Path Normalization

* **`[portal]` Syntax:** Supports standard **bash-like globbing** (`*`, `**`, `?`, `[]`, `{}`).
* **`ignore` Syntax:** Supports **Gitignore-style semantics**.
* **Hidden Files:** Standard behavior applies; `*` does not match hidden files (e.g., `.hidden_file`). Users must explicitly use `.*` patterns to match them.
* **Prefix Normalization:** The `./` prefix is purely cosmetic. `"a" = "b"` and `"./a" = "./b"` are identical.
* **Symlinks in Source:** Symbolic links encountered in the source directory are treated as regular files. The manager does not follow them to resolve their targets during discovery; the symlink itself is deployed.

---

## Validation & Error Handling

### Errors (Halts Execution)
* **Invalid Target Directory:** If `target-dir` is provided but is not a valid absolute path.
* **Path Traversal:** If a resolved target path attempts to escape the defined `target-dir`, or if a source path escapes the source directory.
* **Empty Patterns:** If a `[portal]` key or value is an empty string.
* **Target Collisions:** If two different source paths resolve to the exact same target path. This inherently catches exact duplicate `[portal]` rules after path normalization.

### Warnings (Continues Execution)
* **Unmatched Portal Patterns:** If a `[portal]` glob pattern matches zero files in the source directory (evaluated after `ignore` filtering).
* **Unmatched Rule Patterns:** If a `[rule]` glob pattern matches zero resolved target paths.
* **Invalid Mode on Symlink:** When `[rule]` attempts to apply a `mode` to a `type = "symlink"`.
