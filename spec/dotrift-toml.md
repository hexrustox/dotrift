# `dotrift.toml`

Defines how files are projected from the source directory to the target
directory. The file is discovered at the root of the already-resolved source
directory; it cannot name its own source directory.

The file is required: a missing `dotrift.toml` is a configuration error,
unlike `dotrift_data.toml` and `.dotriftignore`, which are optional.

## Rendering before parsing

Before parsing, the file is evaluated as a template using the run's resolved
variable context (ADR-0001): base variables from `dotrift_data.toml` overlaid
by active profiles (see `spec/dotrift-data-toml.md § Profile resolution`).
The context is resolved once per run and is the same context deployed
templates receive. The render pipeline, in order:

1. Read `dotrift_data.toml` (plain TOML; see `spec/dotrift-data-toml.md`).
2. Read the active-profile selectors from the state database (read-only; a
   missing database contributes no active profiles; a database error halts).
3. Build the variable context.
4. Render `dotrift.toml` with it. The templater's function registry is empty:
   every function call is a template error. No environment variables are
   injected implicitly.
5. Check that the rendered output is valid UTF-8.
6. Parse the rendered text as TOML.

Template errors, missing variables, and non-UTF-8 output fail before TOML
parsing. The rendered result must conform to the structure below.

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
* **Default:** the user's home directory (handled in code; no environment
  expansion in TOML).
* **Description:** Root of the destination tree. The CLI `--target` override
  takes precedence (see `spec/commands/global.md § Target directory
  precedence`). Must be an absolute path.

No other root-level keys are defined. Unknown keys and sections are rejected
as configuration errors. There is no `version` field and no `ignore` field;
ignored paths are configured in a separate `.dotriftignore` file (see
`spec/dotriftignore.md`).

## `[portal]`

Maps source paths to target paths.

* **Keys:** Patterns relative to the source directory, matched against source
  paths.
* **Values:** Literal target paths relative to the target directory. Glob
  metacharacters are not allowed in values.
* **Order:** Portals are processed in sorted-key order. Entry order, and
  which source is named first in a collision error, follow from it.
* **Optional:** A missing or empty table is a valid no-op.

### Portal types

* **Literal keys** (no wildcards) map exactly one source file or directory:
  * A literal key naming a source path that does not exist is a
    configuration error.
  * A literal file maps to exactly one target path. The value `.` is a
    configuration error for a literal file: the target would be the
    target-directory root itself.
  * A literal directory maps recursively; its contents are deployed beneath
    the target destination, preserving descendants. A symlink to a directory
    is mapped like the directory it resolves to. Empty source directories
    produce no deployment entries.
* **Glob keys** (containing wildcards) select matching source files. The
  value is a destination directory; `.` is valid and means the target
  directory root. A wildcard component matches a directory name on the way
  through the tree — traversal follows it, including through symlinked
  directories — but only regular files and symlinks to regular files become
  deployment entries; a directory a glob matches is never itself deployed.
  `*` and `?` match dotfiles like any other name: `config/*` matches
  `config/.hidden`. A glob portal that matches zero source paths is a valid
  no-op.

### Path stripping

For a glob key, the stripping prefix is the portion of the key up to but not
including the first path component that contains a wildcard character —
`*`, `?`, `[`, or `]`. The prefix is removed from the matched source path;
the remainder is appended to the value to form the final target path.

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
    itself; the link target is the absolute source path.
  * `copy` — the link is resolved and the resolved bytes are copied; the
    target is a regular file.
  * `template` — the link is resolved and the resolved bytes are rendered as
    a template; the target is a regular file.
* **Dangling symlinks are errors.** A literal naming a dangling symlink, or a
  glob matching one, is a configuration error. A dangling symlink that no
  portal names or matches is simply not deployed.
* **Cycles are errors.** A symlink cycle encountered during traversal is a
  configuration error. Cycle detection covers the whole traversed tree: a
  glob portal walks the entire source tree, so a cycle anywhere in it fails
  every glob portal, wherever the key points.
* **Paths stay logical.** A file reached through a symlinked directory
  deploys and records under its through-link source path, never a resolved
  one. The same file reachable by two paths deploys twice, to two different
  targets; only target-path *collisions* are rejected.
* **Non-regular resolutions are errors.** A symlink that resolves to neither
  a regular file nor a directory is a configuration error for every deploy
  type. Special source filesystem objects — FIFOs, sockets, device files, and
  the like — are configuration errors for every deploy type.

Every error in this section is a configuration error: it halts execution
before any filesystem change.

## `[rule]`

Selects deployment behavior for resolved target paths.

* **Keys:** Target-relative patterns matched against the resolved target
  path, relative to the target directory. A key without wildcards is an exact
  match: it matches exactly that target path and nothing beneath it. A key
  with wildcards follows the same glob semantics as portal keys: `*` stops at
  `/`, `**` crosses directory boundaries.
* **Values:** Inline tables carrying `type`, `mode`, or both. A rule carrying
  neither (`{}`) is valid and contributes nothing.
* **Order:** Rules are evaluated in declaration order.
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
* `mode`: File permissions, in one of two forms:
  * a string of exactly three octal digits, each `0`–`7` — `"600"`,
    `"755"`; four digits such as `"0600"` are invalid;
  * a TOML integer ≤ `0o777`, interpreted as raw permission bits (`384`,
    `0o600`, and `0x180` denote the same mode).

  Constrained to `000` through `777` in either form. Applies only when the
  effective deploy type is `copy` or `template`. Combining `mode` with an
  explicit `type = "symlink"` in one rule is a configuration error, and so is
  the effective combination: a `mode`-only rule followed by a later rule that
  sets `type = "symlink"` for the same target is a configuration error for
  that target. Omitted means no explicit permission change: the created file
  and any created parent directories receive whatever permissions the process
  umask yields for them.

## Path rules

Applies to portal keys and values and rule keys:

* Paths are root-relative.
* A leading `./` prefix is valid and cosmetic; `./foo` and `foo` are
  equivalent.
* Absolute paths are invalid.
* Embedded `.` or `..` components are invalid, including `/./` and `/../`.
* Empty path components are invalid: `a//b` and a trailing `/` are rejected.
* No path normalization is performed. Target-path equality is nevertheless
  component-wise: `./x` and `x` are the same target path for collision and
  structural-conflict validation.
* No `~` or environment-variable expansion.
* Empty strings are invalid; `.` is the explicit root path — valid as a
  portal source, a glob destination, or a literal-directory destination (see
  [Portal types](#portal-types) for the literal-file exception).
* Glob syntax supports `*`, `**`, `?`, and `[]`. Inside brackets, ranges
  (`[a-z]`) and negation (`[!abc]`) are supported. Brace expansion (`{a,b}`)
  is unsupported; a malformed bracket expression and any other unsupported
  pattern syntax are rejected as configuration errors, not treated literally.

## Validation

Errors halt execution before any filesystem change. Validation runs on the
rendered configuration; rendered values are subject to the same rules as
literal values, with no template-specific exceptions.

* **Invalid target directory:** `target-directory` is not an absolute path.
* **Collision:** any two resolved portal entries producing the same target
  path, including identical declarations for the same source path. The error
  lists the target and the colliding sources/declarations.
* **Structural conflict:** two desired target paths where one is an ancestor
  of the other (for example `config` and `config/editor`) (ADR-0009).
* **Path rule violations:** absolute paths, embedded `.`/`..` components,
  empty components, or empty strings in portal keys/values or rule keys.
* **Missing literal source:** a literal portal key naming a source path that
  does not exist.
* **Unknown fields:** unknown top-level keys, unknown sections, or unknown
  rule properties.
* **Contradictory rule:** `mode` combined with `type = "symlink"`, explicitly
  in one rule or effectively across rules.
* **Invalid mode:** neither allowed form, or outside `000`–`777`.
