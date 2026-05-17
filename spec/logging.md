# Dotrift Message Formatting Standard

This document specifies the formatting rules for all user-facing messages produced by dotrift: errors, warnings, status output, and prompts.

---

## General Rules

* **Sentence case** for all prose messages. Warnings are not exempt.
* **No terminal punctuation** (no `.` `!` at end of messages). Terminal messages are standalone lines, not paragraphs.
* **No leading/trailing whitespace** in message strings (padding is the caller's responsibility).
* Variables interpolated into messages use the placeholder token for the caller (e.g., `{}` for `format!`), never hardcoded in the static string.

---

## Quoting

### Backticks for Paths and Identifiers

Backticks (`` ` ``) delimit all path-like values, file names, executable names, identifiers, and pattern strings within messages.

```
"Failed to open `/etc/config.toml`"
"Source path does not exist: `/home/user/foo`"
"Editor command not found: `vim`"
"Unknown parameter `{name}` in editor command argument"
"Failed to create table `managed_files`"
"Invalid glob pattern: `**/*.rs`"
"Aborted deployment at `/target/path`"
```

### No Quoting for Abstract Concepts

When no specific path or identifier is interpolated, use no quoting.

```
"Failed to apply dotfiles"
"Cannot determine target directory"
"Failed to get current directory"
```

### Bare for Structure that is Markup, not Prose

Environment variable names and other system identifiers that are part of markup-like structure (not prose) appear bare — but only when they are part of a parenthetical formula, not a human-readable path.

```
"Cannot determine data directory ($XDG_DATA_HOME or $HOME not set)"
```

---

## Colon Usage

A colon separates a label from a value only when **the entire message content is the label-value pair** — i.e., ``<Label>: `<value>` ``. If the value sits inside a longer descriptive clause, omit the colon.

```
# Label: value → colon
"Invalid octal mode: `0o999`"
"Invalid ignore pattern: `*.rs`"
"Editor command not found: `vim`"
"Unknown parameter: `{name}`"

# Value embedded in a clause → no colon
"Mode `0o999` exceeds 777"
"File already exists at `/foo/bar`"
"Target directory `/tmp/t` cannot be inside source directory `/tmp/s`"
```

---

## Message Categories

### Failure Messages

**Pattern:** ``Failed to <action> `<path>` `` or `Failed to <action>` (no path) or ``Failed to <action> `<path>`: <reason> `` (with reason).

```
"Failed to open `/file`"
"Failed to copy `/src` to `/dst`"
"Failed to read from `/file`"
"Failed to list managed files"
"Failed to expand editor command parameters"
```

### Precondition Messages

**Pattern:** `Cannot <condition>` or `<Statement of missing precondition>`.

```
"Cannot determine target directory"
"Cannot determine config directory ($XDG_CONFIG_HOME or $HOME not set)"
"Cannot insert empty target path"
```

### Validation Messages

**Pattern:** ``Invalid <thing>: `<value>` `` or `<descriptive sentence>`.

```
"Invalid octal mode: `0o999`"
"Mode `0o999` exceeds 777"
"Invalid ignore pattern: `*.rs`"
"Target directory must be an absolute path: `/relative/path`"
```

### State Messages

**Pattern:** `<statement of fact about current state>`.

```
"File already exists at `/foo/bar`"
"Directory exists when creating file at `/foo`"
"File exists when creating directory at `/foo`"
"Target path collision at `/foo`:"
```

### Abort Messages

**Pattern:** ``Aborted <action> at `<path>` `` or `Aborted <action>`.

```
"Aborted deployment at `/target/path`"
```

### Warning Messages

**Pattern:** `<descriptive sentence>, skipping`. Same formatting rules as errors except they are routed through `print_warn`.

```
"Destination `/path` is outside source directory, skipping"
"`/path` already exists, skipping"
"`/path` not in database, skipping"
```

---

## Multi-line Messages

Summary line ends with a colon. Subsequent lines are indented by 2 spaces.

```
"Target path collision at `/foo`:"
"  Source 1: `/a/foo`"
"  Source 2: `/b/foo`"
```
