# `dotrift.toml`

Defines the file mapping, filtering, and deployment rules. The file is
evaluated as a template before being parsed — see `spec/templater.md` for the
template syntax. The evaluated result must be valid TOML conforming to the
structure below. Template context is resolved from `dotrift_data.toml` (see
`spec/dotrift-data-toml.md`).

```toml
# Optional root-level keys
target-directory = "/absolute/path"
ignore = ["pattern1", "pattern2"]

[portal]
"source_pattern" = "target_path"

[rule]
"target_pattern" = { type = "symlink", mode = "600" }
```

---

## Root Keys

### `target-directory`

* **Type:** String (absolute path)
* **Default:** `$HOME` (handled in code, not via TOML env var expansion)
* **Description:** The root directory where files will be mapped to. If omitted, the application defaults to the user's home directory. Can be overridden via CLI argument. Precedence rules live in `spec/core.md` under Target Directory Precedence.

### `ignore`

* **Type:** Array of strings
* **Default:** `[]`
* **Description:** A list of patterns defining deployment targets to exclude. This is useful for temporarily disabling a mapping or resolving ambiguities when one source file is mapped to multiple locations.
* **Syntax:** Follows exact **Gitignore-style semantics** (including `!` for negation and trailing `/` for directory-only matching). Order matters — `!` patterns re-include previously excluded paths.
* **Matching Context:** Matched against the **resolved target path**, relative to `target-directory`.
* **Implicit Ignores:** `dotrift.toml` and `dotrift_data.toml` are implicitly excluded from deployment. They match no portal and are never written to the target directory.

---

## `[portal]`

Defines the routing of files from the source directory to the target directory.

* **Keys:** Bash-like glob patterns, relative to the source directory, matched against the source path.
* **Values:** The destination path relative to the target directory.
* **Mapping Types:**
  * **Literal Keys:** If the key is a literal path (no wildcards), it maps exactly one source file or directory to the exact target path. (e.g., `"config.ini" = ".config/app/config.ini"`).
  * **Glob Keys:** If the key contains a wildcard, the value must be a directory path. The stripped remainder of the source path is appended to this directory value to form the final target path.

If a file matches multiple keys, they will all be applied if not colliding at the target directory.

### Path Stripping Rule

<a id="path-stripping-rule"></a>

When a `[portal]` key contains a wildcard, the target path is calculated by stripping a prefix from the matched source path.

The **stripping prefix** is the portion of the key *up to but not including* the first path component that contains any wildcard character (`*`, `?`, or `[]`). If the very first path component contains a wildcard, the stripping prefix is empty.

This prefix is removed from the beginning of the source path that matched the glob. The remainder is then appended to the value specified in the `[portal]` table.

**Literal keys** (those containing no wildcards) are not subject to this rule — they map the source path exactly to the target path given.

### Examples

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
  → Source file `foo/bar.rs` → after stripping → `foo/bar.rs` → appended to `"."` → final target = `./foo/bar.rs` (maps the entire source directory to the target directory).

* **Glob (Filename wildcard only):** `"conf*.ini" = "settings"`
  → Path components of key: `conf*.ini`.
  → First (and only) component contains a wildcard.
  → Stripping prefix = `""` (empty).
  → Source file `config.ini` → after stripping → `config.ini` (full filename is kept) → final target = `settings/config.ini`.

### Recursion & Multi-Match

* Literal directories and globs like `"dir/**"` recurse into directories, mapping contents.
* Empty directories in source are ignored (no target mapping).
* A source file or directory can match multiple portals, mapping to multiple targets if no target path collision.

---

## `[rule]`

Defines the deployment method (`type`) and file permissions (`mode`); directory rule is not supported.

* **Keys:** Bash-like glob patterns matched against the **resolved target path**, relative to the target directory.
* **Values:** Object containing `type` and/or `mode`.
* **Scope:** **File-only.** No implicit recursive directory matching.
* **Precedence:** Evaluated in exact configuration order (guaranteed via `indexmap`). The properties are shallow-merged. Last rule wins on conflict.
* **Empty Tables:** Empty `[portal]` or `[rule]` tables are valid and treated as no-ops.

### Properties

* `type` (String): `"symlink"`, `"copy"`, or `"tmpl"`. Defaults to `"symlink"` if no rule matches. The set of values is the *deploy type* enum (see `CONTEXT.md`).
  * **tmpl:** The source file is evaluated as a template (see `spec/templater.md`) before being written to the target.
* `mode` (String): File permissions represented as an octal string of digits only (e.g., `"600"`, `"0600"`). No `0o` prefix. Must be a valid octal value in the range `000`–`777` (error otherwise). Defaults to none (no explicit modification) if omitted.

### Source Symlink Behavior

<a id="source-symlink-behavior"></a>

When a source path is itself a symbolic link, dotrift never follows it during
discovery — the symlink *is* the file being managed. The deploy type controls
what target gets created, and the three deploy types diverge for source
symlinks:

* **`copy`** of a source symlink → the target path becomes a symlink pointing to the *same destination* as the source symlink (identity preservation: source→`/a/b`, target→`/a/b`).
* **`symlink`** of a source symlink → the target path becomes a symlink pointing to the *source symlink itself* (indirection back into the source directory: source→`/a/b`, target→source).
* **`tmpl`** → the source symlink chain is followed to its ultimate resolution, the resolved file's content is read as a template, rendered, and the output is written as a regular file at the target path (content resolution: source→`/a/b`, target reads `/a/b`, renders template, writes result).

The asymmetry is intentional. `copy` means "preserve what the user has on
disk (a link to somewhere else)." `symlink` means "indirect target readers
back to my source directory." `tmpl` means "render this file's content" — which
requires following the link to reach actual bytes. The decision record is
ADR-0002.

---

## Globbing

* **`[portal]` Syntax:** Supports standard **bash-like globbing** (`*`, `**`, `?`, `[]`). Uses `glob` crate. Brace expansion (`{}`) is not supported.
* **`ignore` Syntax:** Supports **Gitignore-style semantics**.
* **Prefix Normalization:** The `./` prefix is purely cosmetic. `"a" = "b"` and `"./a" = "./b"` are identical. Absolute paths in `[portal]` keys (e.g., `"/etc/foo"`) are normalized as relative to the source directory (e.g., `"./etc/foo"`).
* **Path Normalization Clamping:** Portal keys are normalized as relative paths. `../` components in a relative path resolve against the path's own root — leading `..` components that would "escape" are preserved but cannot reference above the root of a relative path. When joined with the source directory or target directory, the resulting absolute path stays within both. The same normalization applies to target-side paths. No explicit clamping is needed; path normalization provides this guarantee.

---

## Validation & Errors

Errors halt execution.

* **Invalid Target Directory:** If `target-directory` is provided but is not a valid absolute path.
* **Source-Target Overlap:** Error if the source directory equals the target directory (prevents self-modification loops).
* **Target Inside Source:** Error if the target directory is inside the source directory (prevents self-modification).
* **Target Path Inside Source:** Error if any resolved target path is within the source directory (prevents dotfiles from being mapped back into the source directory).
* **Target Collisions:** If two different source paths resolve to the exact same target path. Show all colliding source paths and the collision target. Halts program.
