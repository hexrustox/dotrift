# Global options

The global CLI surface shared across every subcommand: the source directory,
the target directory, and how path-supplying arguments are resolved.
Subcommand specs reference this file rather than restating these rules.

**Usage:** `dotrift [GLOBAL OPTIONS] <COMMAND> [ARGS]`

## Global options

### `-s, --source <dir>`

* **Type:** path.
* **Default:** `$XDG_DATA_HOME/dotfiles`, falling back to
  `$HOME/.local/share/dotfiles` when `XDG_DATA_HOME` is unset. An error is
  raised only when both `XDG_DATA_HOME` and `HOME` are unset.
* **Description:** Overrides the source directory. The source directory is
  resolved before `dotrift.toml` is read, and the configuration cannot set it.
  A source directory that is itself a symlink is followed: portal resolution
  walks the directory it resolves to (see `../dotrift-toml.md`, Source
  symlink behavior).

### `-t, --target <dir>`

* **Type:** path.
* **Default:** `$HOME`. An error is raised when `$HOME` is unset or empty.
* **Description:** Overrides the target directory. The resolved value must be
  absolute. A target directory that is itself a symlink is followed when it
  resolves to a directory; one that resolves to a non-directory or dangles is
  an error at preflight (see `apply.md`).

No other global options are defined.

## Path resolution

Any flag or argument that supplies a path may be given as a relative path. It
is resolved against the process's current working directory to an absolute
path immediately after CLI parsing. This applies to every path-supplying
argument across all subcommands, not just the global options.

A `--source` or `--target` root that is itself a symlink is followed: the
root is used through the directory it resolves to. Roots otherwise keep the
logical path as supplied; the one place they are resolved further is the
source/target overlap comparison, which canonicalizes both roots before
comparing them (ADR-0012).

## Target directory precedence

When the target directory is needed, it is resolved in order:

1. `--target` CLI option, if provided.
2. `target-directory` in `dotrift.toml`, if provided (must be absolute; see
   `dotrift-toml.md`).
3. `$HOME`.

When `--target` is provided, the CLI override wins outright: the config-side
`target-directory` value is unused and its absolute-path rule is not enforced.
The config file is still parsed in full for its other content, so a malformed
file — including a structurally invalid `target-directory` — fails regardless
of the override.

## Source directory requirement

A command that reads the control files (`dotrift.toml`, `dotrift_data.toml`,
`.dotriftignore`) errors if the resolved source directory does not exist.

## Output conventions

Shared presentation rules for all command output. Subcommand specs reference
this section rather than restating these rules.

### Color support

Color support is detected once, at startup, against standard output, using the
standard `supports-color` conventions: colors are enabled when stdout is a
terminal and not disabled by the standard environment variables (for example
`NO_COLOR`); they are forced on by the standard force variables even when
stdout is not a terminal. Every command gates all of its coloring on that
single decision — there is no per-command or per-line opt-out, and no
color-detection after startup.

Coloring never changes text content or layout: with colors disabled, the
output is byte-identical plain text with the same words in the same order.
Only the presentation of individual words changes.

The pinned palette:

| Word | Color |
|---|---|
| `managed` | green |
| `unmanaged` | red |
| `deployed` | green |
| `replaced` | cyan |
| `skipped` | dark grey |
| `removed` | red |
| `pruned` | magenta |
| `obstruction` | yellow |
| `(active)` | green |

### Path display

Paths shown in output are prettified: a path under the user's home directory
is displayed with a leading `~` replacing the home prefix (the bare home
directory displays as `~`); any other path is displayed as its normalized
absolute form. This applies to every path `status`, `apply`, and `profile`
print — verbose lines, dry-run lines, clean-up lines, and obstruction prompts
alike. Prettification is display-only; state records and comparisons always
use the real absolute paths.
