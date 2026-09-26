# `.dotriftignore`

Defines which resolved target paths are excluded from deployment. The file is
discovered at the root of the already-resolved source directory, alongside
`dotrift.toml` and `dotrift_data.toml`. It is optional: a missing file
contributes no user-defined ignore patterns.

Unlike `dotrift.toml`, the file is plain text and is never evaluated as a
template. No `dotrift_data.toml` variables, profiles, or environment
expansion are available. Patterns use standard gitignore syntax (ADR-0002).

## Discovery

* **Location:** root of the resolved source directory. The file is never
  looked for elsewhere.
* **Optional:** a missing file contributes no patterns. An empty file is
  valid. A dangling symlink at `.dotriftignore` counts as missing.
* **Errors:** an unreadable file — an I/O error, or the path being a
  directory — halts execution before any deployment. Missing is the
  only absence treated as "no ignore patterns".

## Pattern syntax

Each line is one ignore pattern. Blank lines and lines whose first character
is `#` are ignored. Trailing whitespace is stripped from each line before it
is treated as a pattern; a space survives only when escaped with a backslash
(a line ending in `\ `), matching gitignore behavior.

The standard gitignore pattern forms are supported:

* `*` — matches any sequence of non-`/` characters.
* `?` — matches exactly one non-`/` character.
* `**` — matches any number of directories, including none.
* `[abc]` / `[!abc]` — character classes.
* Leading `/` — anchors the pattern to the target-directory root.
* Trailing `/` — recognized as a directory-only pattern in gitignore syntax,
  but inert: dotrift matches only resolved file entries (§ Matching), so such
  a pattern matches nothing. To exclude a directory's contents, use `dir/**`
  (or `**/dir/**` to match at any depth).
* Leading `!` — negation; re-includes a target path previously ignored by an
  earlier pattern.
* A leading `\#` or `\!` is not a comment or a negation: the backslash escapes
  the character, is dropped, and the pattern begins with a literal `#` or `!`.
* A slash anywhere else in the pattern anchors it relative to the
  target-directory root (gitignore behavior).

An unclosed bracket expression (for example `[abc`) is treated literally,
matching established gitignore behavior. A pattern that cannot be compiled as
a glob at all — an invalid character range, for example — is a configuration
error; it is not silently dropped.

## Matching

Ignore patterns match target paths, not source paths (ADR-0002).

* **Match subject:** each resolved portal entry's target path, relative to
  the target directory, using `/` separators. Only deployment entries are
  tested, and every deployment entry is a file.
* **Anchoring:** full gitignore semantics. A pattern containing no slash
  matches the entry's basename at any depth — `foo` matches `a/foo` and
  `a/b/foo`. A pattern containing a slash anywhere (leading or otherwise) is
  anchored to the target-directory root.
* **Case sensitivity:** matching is case-sensitive, regardless of the target
  filesystem's case behavior.
* **Directories:** only resolved files are tested; directories are not
  deployment entries. A trailing-`/` pattern therefore never matches a target
  path — it excludes neither the named directory nor the files beneath it.
  This is accepted behavior: a directory-only pattern is silently inert. Use
  `dir/**` to exclude a directory's contents.
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

The implicit patterns are root-anchored target paths: any entry whose target
path at the target-directory root is one of these three names is ignored,
regardless of which source file was mapped onto it. A portal that maps a
control file to a nested target path — for example
`"dotrift.toml" = "settings/dotrift.toml"` — deploys it: this is not a
general ban on deploying the control files. This is intentional.

Files with the same names nested below the source root are ordinary source
files and remain deployable.

## Validation

* **Unreadable file:** an I/O error halts execution before any deployment.
* **Non-compilable pattern:** a configuration error, halting before any
  deployment.
* **Missing file:** valid; no user-defined patterns.
