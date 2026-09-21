# `init`

Creates an initialized source directory: the directory itself, plus the three
control files — `dotrift.toml`, `dotrift_data.toml`, and `.dotriftignore` —
with commented-out, inert example content. `init` writes the source directory;
it never reads the control files, opens the state database, or interacts with
the target directory or the global config.

**Usage:** `dotrift init`

`init` takes no arguments and no subcommands.

## Pipeline

1. Resolve the source directory per the global `-s` rules. Unlike commands
   that read the control files, a missing source directory is not an error:
   creating it is `init`'s purpose, so the source-directory requirement (see
   `spec/commands/global.md § Source directory requirement`) does not apply.
2. If the resolved source path exists but is not a directory — a file, or a
   symlink that dangles or resolves to a non-directory — error.
3. If it does not exist, create it, including missing parent directories. A
   symlink resolving to a directory is used as-is (see
   `spec/commands/global.md § Global options`).
4. If `dotrift.toml` exists at the source root, error: the source directory
   is already initialized. Nothing is created or modified.
5. Create each missing control file with its scaffold content (§ Scaffold).
   A file already present is left untouched.

## Presence

A control file is present when a directory entry of any kind exists at its
path, without following symlinks: a dangling symlink at a control-file path
counts as present, and creation never writes through one.

## Scaffold

Each created control file contains only comments, so the scaffold is valid
and inert: `dotrift.toml` parses as TOML with no portals and no rules, the
data file defines no variables or profiles, and the ignore file contributes
no patterns. Because `dotrift.toml` is rendered as a template before parsing
(see `spec/dotrift-toml.md § Rendering before parsing`), its scaffold contains
no template tags.

The created files' exact content:

`dotrift.toml`:

```toml
# Maps source paths to target paths. Rendered as a template before
# parsing: template tags in this file are evaluated, so keep examples
# commented out unless you want them rendered.

# target-directory = "/absolute/path"

# [portal]
# "config/**/*.toml" = ".config"
# "file1" = ".file1"

# [rule]
# ".config/**" = { type = "copy" }
# ".config/secrets/**" = { mode = "600" }
```

`dotrift_data.toml`:

```toml
# Base variables and profiles for template rendering.

# [variable]
# str = "str"
# num = 1

# [profile.profile1]
# str = "profile1"
```

`.dotriftignore`:

```
# Excludes resolved target paths from deployment. Gitignore-style
# patterns: a pattern containing no slash matches a file name at any
# depth; a pattern containing a slash is anchored to the
# target-directory root.

# file1
# dir1/**
```

## Output

On success, one line per created control file, in the order `dotrift.toml`,
`dotrift_data.toml`, `.dotriftignore`, each the file's path displayed per
`spec/commands/global.md § Output conventions`. Files left untouched print
nothing; errors print nothing to standard output.

## Concurrency

`init` touches no state: it never opens the state database and never acquires
the state lock.

## Exit status

`init` exits `0` on success and `1` on error: an already-initialized source
directory, a source path that is not a directory, or a filesystem failure.
Usage errors exit `2` (see `spec/commands/global.md § CLI conventions`).
