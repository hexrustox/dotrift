# `.dotriftignore`

Defines which resolved target paths are excluded from deployment. The file is
discovered at the root of the already-resolved source directory, alongside
`dotrift.toml` and `dotrift_data.toml`. It is optional: a missing file
contributes no user-defined ignore patterns.

Unlike `dotrift.toml`, the file is plain text and is never evaluated as a
template. No `dotrift_data.toml` variables, profiles, or environment
expansion are available. Patterns use standard gitignore syntax.

## Discovery

* **Location:** root of the resolved source directory. The file is never
  looked for elsewhere.
* **Optional:** a missing file contributes no patterns. An empty file is
  valid.
* **Errors:** an unreadable or malformed file halts execution before any
  filesystem change. Missing is the only absence treated as "no ignore
  rules".

## Pattern syntax

Each line is one ignore pattern. Blank lines and lines whose first
non-whitespace character is `#` are ignored.

The standard gitignore pattern forms are supported:

* `*` — matches any sequence of non-`/` characters.
* `?` — matches exactly one non-`/` character.
* `**` — matches any number of directories, including none.
* `[abc]` / `[!abc]` — character classes.
* Leading `/` — anchors the pattern to the target-directory root.
* Trailing `/` — directory-only pattern; matches that directory and all its
  descendants.
* Leading `!` — negation; re-includes a target path previously ignored by an
  earlier pattern.
* A slash anywhere else in the pattern anchors it relative to the
  target-directory root (gitignore behavior).

A pattern that cannot be parsed as valid gitignore syntax is a configuration
error; it is not treated literally.

## Matching

Ignore patterns match target paths, not source paths.

* **Match subject:** each resolved portal entry's target path, relative to the
  target directory, using `/` separators.
* **Case sensitivity:** matching is case-sensitive, regardless of the target
  filesystem's case behavior.
* **Directories:** a trailing-`/` pattern matches the named directory and all
  deployed files beneath it. Directories are not deployment entries; only
  resolved files are tested.
* **Order:** patterns are evaluated in file order. When several patterns
  match, the last matching pattern decides whether the target path is ignored.
  A later `!` pattern re-includes paths matched by earlier patterns.

## Filtering stage

Ignoring is applied after portal resolution and before collision validation,
rule evaluation, and deployment.

* Each resolved portal entry is tested independently against its own target
  path. A source path mapped by several portals may be ignored for one target
  while still deployed to another.
* Ignored entries are removed before collision validation; an ignored entry
  never causes a collision.
* Rules and deployment never observe ignored entries.

## Implicit ignore patterns

The root control files are implicitly excluded. These patterns are evaluated
first, before the file's own patterns:

* `/dotrift.toml`
* `/dotrift_data.toml`
* `/.dotriftignore`

Because the file's patterns are evaluated after and the last match wins, a
negation such as `!dotrift.toml` re-includes a control file.

The implicit exclusion applies only to these three filenames at the source
root. Files with the same names nested below the source root are ordinary
source files and remain deployable.

## Validation

* **Unreadable file:** an I/O error halts execution before any filesystem
  change.
* **Malformed pattern:** a configuration error, halting before any filesystem
  change.
* **Missing file:** valid; no user-defined patterns.
