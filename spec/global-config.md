# Global Configuration

Loaded from `$XDG_CONFIG_HOME/dotrift/config.toml` (overridable via the
global `-c, --config <FILE>` option, see `spec/core.md`). This file is
optional; a missing file is treated as empty (all defaults apply). Invalid
or malformed TOML is an error. Partial configs merge with defaults.

```toml
overwrite-identical = false

[editor-command]
command = "vim"
args = ["-f"]
```

---

## Fields

### `overwrite-identical`

* **Type:** bool
* **Default:** `false`
* **Description:** Whether to update the DB entry when a target file already matches what dotrift would write. Consulted by `apply` during the Identical Check (see `spec/commands/apply.md`).

### `[editor-command]`

Optional table. Command to open `dotrift.toml` for the `add` command (see
`spec/commands/add.md`).

* `command` (string): The executable name or path.
* `args` (array of strings): Arguments passed to the command. Supports parameter expansion (see below).

If `editor-command` is omitted, `add` falls back to `$VISUAL`, then `$EDITOR`.

---

## Parameter Expansion

The `args` array supports parameter expansion using `{param}` syntax.

| Parameter | Description |
|-----------|-------------|
| `{file}` | Absolute path to the file being opened |
| `{row}` | Line number (1-indexed) |
| `{col}` | Column number (1-indexed) |

**Rules:**

* `{param}` in args strings is replaced with the parameter value.
* All parameters are guaranteed to be set by the program.
* Unknown parameter: error.
* Literal braces: `{{` and `}}` produce `{` and `}`.
* No shell expansion — args passed directly to `Command::new()`.

**Example:**

```toml
[editor-command]
command = "vim"
args = ["-f", "{file}", "+{row}"]
```

Expands to: `vim -f /path/to/dotrift.toml +42`