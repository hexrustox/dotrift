# Global config

An optional, per-user TOML file that configures dotrift across all source
directories. It is distinct from the *control files* (see `spec/CONTEXT.md §
Control file`), which live in the source directory and configure a single
deployment; the global config configures dotrift itself.

Unlike `dotrift.toml`, the file is never evaluated as a template: no
variables, profiles, or environment expansion are available. It is parsed as
plain TOML (ADR-0018).

```toml
[pager]
command = "less"
args = ["-R"]

[apply]
replace-identical = false
```

## Discovery

* **Location:** `$XDG_CONFIG_HOME/dotrift/config.toml`, falling back to
  `$HOME/.config/dotrift/config.toml` when `XDG_CONFIG_HOME` is unset. An
  error is raised only when no home directory can be resolved at all.
* **Optional:** a missing file contributes the defaults: no pager configured,
  `replace-identical` off. A dangling symlink at the config path counts as
  missing.
* **Errors:** an unreadable file — an I/O error, or the path being a
  directory — is an error, as is malformed TOML. Missing is the only absence
  treated as "no configuration".

## `[pager]`

Selects the pager dotrift displays diffs through. The table is optional; a
missing table configures no pager. When the table is present it must carry
`command`.

* **`command`** (String, required when the table is present): the pager
  program, used verbatim. No whitespace splitting is performed — a value like
  `"less -R"` would look for a program literally named `less -R`; arguments
  belong in `args`. An empty or whitespace-only value is valid and counts as
  unset, falling through the pager priority (see `spec/commands/apply.md §
  Obstruction prompts`).
* **`args`** (Array of String, optional, default empty): literal arguments
  appended after the program. Whitespace inside an argument is preserved;
  arguments are never re-split.

## `[apply]`

Application-time behavior. The table is optional; a missing or empty table
leaves every property at its default.

* **`replace-identical`** (Bool, optional, default `false`): when `true`,
  `apply` replaces identical obstructions without prompting (see
  `spec/commands/apply.md § Identical obstructions`).

## Consumption

* **Who reads it:** `apply` reads the global config once per run, after
  acquiring the state lock and before reading the control files. No other
  command reads it today.
* **Failure timing:** any discovery or validation error fails the run before
  any filesystem change, including under `--dry-run`. The config is read
  eagerly (ADR-0018): a broken global config fails every `apply` run, even
  one whose settings would not have been consulted.

## Validation

Errors halt execution before any filesystem change.

* **Malformed TOML:** a parse error halts execution.
* **Unknown structure:** any root table or key other than `[pager]` and
  `[apply]`, or any property inside them other than the ones defined above,
  is rejected. Typos fail rather than silently leaving the setting
  unapplied.
* **Wrong types:** `command` that is not a string, `args` that is not an
  array of strings, or `replace-identical` that is not a boolean.
* **Missing `command`:** a `[pager]` table without `command`.

There is no per-source override of these settings in `dotrift.toml`, and no
CLI option overrides them.
