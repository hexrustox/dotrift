# `dotrift.toml`

Defines how files are projected from the source directory to the target
directory. The file is discovered at the root of the already-resolved source
directory; it cannot name its own source directory.

Before parsing, the file is evaluated as a template (see the Templater
context) using the same resolved variable context as deployed templates: base
variables from `dotrift_data.toml` overlaid by active profiles in precedence
order. The rendered result must be valid TOML conforming to the structure
below. Template errors and missing variables fail before TOML parsing. No
environment variables are injected implicitly.

```toml
# Optional root-level keys
target-directory = "/absolute/path"

[portal]
"source/path/or/glob" = "target/path"

[rule]
"target/path/or/glob" = { type = "symlink" }
```

## Root keys

### `target-directory`

* **Type:** String (absolute path).
* **Default:** `$HOME` (handled in code; no environment expansion in TOML).
* **Description:** Root of the destination tree. The CLI `--target` override
  takes precedence. Must be an absolute path.

No other root-level keys are defined. Unknown keys and sections are rejected
as configuration errors. There is no `version` field and no `ignore` field;
ignored paths are configured in a separate `.dotriftignore` file (see
[`dotriftignore.md`](dotriftignore.md)).

## `[portal]`

Maps source paths to target paths.

* **Keys:** Patterns relative to the source directory, matched against source
  paths.
* **Values:** Literal target paths relative to the target directory. Glob
  metacharacters are not allowed in values.
* **Optional:** A missing or empty table is a valid no-op.

### Portal types

* **Literal keys** (no wildcards) map exactly one source file or directory:
  * A literal file maps to exactly one target path.
  * A literal directory maps recursively; its contents are deployed beneath
    the target destination, preserving descendants. A symlink to a directory
    is mapped like the directory it resolves to. Empty source directories
    produce no deployment entries.
* **Glob keys** (containing wildcards) select matching source files. The
  value is a destination directory.

### Path stripping

For a glob key, the stripping prefix is the portion of the key up to but not
including the first path component that contains a wildcard character. The
prefix is removed from the matched source path; the remainder is appended to
the value to form the final target path.

* `"config/**/*.toml" = ".config"` — source `config/app/settings.toml` maps
  to `.config/app/settings.toml`.
* `"**" = "."` — the source root maps to the target root; no stripping prefix.

### Recursion and multi-match

Literal directories and `dir/**` globs recurse, including through symlinked
directories (see [Source symlink behavior](#source-symlink-behavior)). A
source path may match multiple portals, producing multiple targets. Empty
source directories are ignored.

## Source symlink behavior

Source symlinks are transparent during portal resolution: traversal follows
them, whatever form the portal key takes. This section is the single
statement of source-side symlink semantics; the command and global-option
specs reference it rather than restate it.

* **Traversal follows.** Globs descend into symlinked directories, and a
  literal key may traverse symlinked path components. A symlink to a
  directory is treated as the directory it resolves to: a literal naming one
  maps recursively, and a glob reaching one descends into it. A symlinked
  directory is never itself a deployable entry.
* **Symlinked files deploy.** A source path that is a symlink to a regular
  file is a deployable entry like any regular file, and the deploy type
  controls what the target becomes:
  * `symlink` — the target path becomes a symlink pointing to the source path
    itself.
  * `copy` — the link is resolved and the resolved bytes are copied; the
    target is a regular file.
  * `template` — the link is resolved and the resolved bytes are rendered as
    a template; the target is a regular file.
* **Dangling symlinks are errors.** A literal naming a dangling symlink, or a
  glob matching one, is a configuration error. A dangling symlink that no
  portal names or matches is simply not deployed.
* **Cycles are errors.** A symlink cycle encountered during traversal is a
  configuration error.
* **Paths stay logical.** A file reached through a symlinked directory
  deploys and records under its through-link source path, never a resolved
  one. The same file reachable by two paths deploys twice, to two different
  targets; only target-path *collisions* are rejected.
* **Non-regular resolutions are errors.** A symlink that resolves to neither
  a regular file nor a directory is a configuration error for every deploy
  type.

Every error in this section is a configuration error: it halts execution
before any filesystem change.

## `[rule]`

Selects deployment behavior for resolved target paths.

* **Keys:** Target-relative patterns matched against the resolved target
  path, relative to the target directory.
* **Values:** Inline tables carrying `type` and/or `mode`.
* **Scope:** File-only. A rule never configures directory permissions and
  never creates a directory deployment entry. Patterns may contain directory
  components.
* **Optional:** A missing or empty table is a valid no-op.

### Precedence

Rules are evaluated in declaration order. When several rules match, later
rules override earlier values property-by-property. Last matching rule wins on
conflict per property.

```toml
[rule]
"config/**" = { type = "copy" }
"config/secrets/**" = { mode = "600" }
```

A file at `config/secrets/x` resolves to `copy` with mode `600`.

### Properties

* `type` (String): `"symlink"`, `"copy"`, or `"template"`. Defaults to
  `"symlink"` when no matching rule sets a type. The set of values is the
  *deploy type* enum.
* `mode` (String): File permissions as a quoted three-digit octal string such
  as `"600"` or `"755"`. Four-digit values such as `"0600"` are invalid.
  Constrained to `000` through `777`. Applies only when the effective deploy
  type is `copy` or `template`; combining `mode` with `type = "symlink"` is a
  configuration error. Omitted means no explicit permission change.

## Path rules

Applies to portal keys and values and rule keys:

* Paths are root-relative.
* A leading `./` prefix is valid and cosmetic; `./foo` and `foo` are
  equivalent.
* Absolute paths are invalid.
* Embedded `.` or `..` components are invalid, including `/./` and `/../`.
* No path normalization is performed.
* No `~` or environment-variable expansion.
* Empty strings are invalid; `.` is the explicit root path.
* Glob syntax supports `*`, `**`, `?`, and `[]`. Brace expansion (`{a,b}`) is
  unsupported; unsupported pattern syntax is rejected, not treated literally.

## Validation

Errors halt execution before any filesystem change. Validation runs on the
rendered configuration; rendered values are subject to the same rules as
literal values, with no template-specific exceptions.

* **Invalid target directory:** `target-directory` is not an absolute path.
* **Collision:** any two resolved portal entries producing the same target
  path, including identical declarations for the same source path. The error
  lists the target and the colliding sources/declarations.
* **Path rule violations:** absolute paths, embedded `.`/`..` components, or
  empty strings in portal keys/values or rule keys.
* **Unknown fields:** unknown top-level keys, unknown sections, or unknown
  rule properties.
* **Contradictory rule:** `mode` combined with `type = "symlink"`.
* **Invalid mode:** not a three-digit octal string, or outside `000`–`777`.
