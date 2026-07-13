# dotrift

A declarative, template-aware dotfile manager written in Rust. dotrift maps files from a single source directory to a target directory (typically `$HOME`) using a plain `dotrift.toml` — symlinking some files, copying others, and rendering templates for the rest with a custom-built templating language and a dedicated templater workspace crate. A SQLite-backed state database tracks what dotrift manages, detects external modifications, and surfaces conflicts interactively in a TUI pager before overwriting anything. A profile system layers environment-specific variables over a base `dotrift_data.toml` so the same config can produce different outputs across machines without forking the repo.

## Features

- **Declarative configuration** — Plain TOML defines source-to-target mappings with glob patterns
- **Three deploy types** — Symlink, copy, or template rendering per file or pattern
- **Template engine** — Custom templating language with conditionals, loops, functions, and whitespace control
- **Profile system** — Layer environment-specific variables (work vs personal, Linux vs macOS) over base config
- **State tracking** — SQLite database tracks managed files, detects external modifications, prevents accidental overwrites
- **Interactive conflict resolution** — TUI pager with side-by-side diffs, directory explorer, and file viewer
- **Collision detection** — Halts with clear error when multiple sources map to the same target
- **Dry-run mode** — Preview planned operations before touching the filesystem

## Installation

Build from source:

```bash
git clone <repo-url>
cd dotrift
cargo build --release
```

The binary will be at `target/release/dotrift`. Move it to a directory in your `$PATH`.

**Requirements:** Rust 1.95 or later (Edition 2024).

## Quick Start

Initialize the source directory:

```bash
dotrift init
```

This creates `~/.local/share/dotrift/dotrift.toml` (or `$XDG_DATA_HOME/dotrift/dotrift.toml`).

Edit the config to define your mappings:

```toml
# Map entire source root to home directory
"**" = "."

# Or map specific files
"bashrc" = ".bashrc"
"config/git" = ".config/git"
```

Apply the configuration:

```bash
dotrift apply
```

Files are now symlinked, copied, or rendered to your home directory.

## Usage

| Command | Description |
|---------|-------------|
| `init` | Initialize source directory with default `dotrift.toml` |
| `apply` | Evaluate config and deploy files to target directory |
| `unapply` | Remove all managed files from target directory |
| `add <path> [dest]` | Add existing file to source directory and update config |
| `diff <path>` | Show side-by-side diff between managed file and source |
| `status list [file]` | List managed files or check specific file status |
| `status clear [file]` | Clear status from database (files remain on disk) |
| `profile list` | Show all profiles, mark active ones |
| `profile activate <name>` | Activate a profile (updates timestamp) |
| `profile deactivate <name>` | Deactivate a profile |
| `profile show` | Print resolved variable context |
| `templater` | Evaluate template standalone (see `dotrift templater --help`) |

**Global options:**

- `-s, --source <dir>` — Override source directory (default: `$XDG_DATA_HOME/dotrift`)
- `-t, --target <dir>` — Override target directory (default: `$HOME`)
- `-c, --config <file>` — Override config file path
- `-v, --verbose` — Enable verbose logging

See `dotrift <command> --help` for command-specific options.

## Configuration

`dotrift.toml` defines file mappings and deployment rules. Before parsing, it's evaluated as a template (see [Template Syntax](#template-syntax)).

### Mapping with `[portal]`

Maps source files to target paths using glob patterns:

```toml
[portal]
# Literal file mapping
"bashrc" = ".bashrc"

# Literal directory (recursive)
"config/git" = ".config/git"

# Glob pattern (maps entire subtree)
"config/**" = ".config"

# Map source root to target root
"**" = "."
```

**Path stripping:** For glob keys, the prefix up to the first wildcard component is stripped from matched paths. Example: `"src/**/*.rs" = "dist"` maps `src/foo/bar.rs` to `dist/foo/bar.rs`.

### Deployment rules with `[rule]`

Controls deploy type and file permissions:

```toml
[rule]
# Render as template
"*.tmpl" = { type = "tmpl" }

# Copy with specific permissions
"scripts/*" = { type = "copy", mode = "755" }

# Symlink (default if no rule matches)
"config/**" = { type = "symlink" }
```

**Deploy types:**
- `symlink` — Create symbolic link to source file (default)
- `copy` — Copy file content
- `tmpl` — Render source as template, write output

### Excluding files with `ignore`

Gitignore-style patterns to exclude files from deployment:

```toml
ignore = ["*.tmp", "secrets/**", "!secrets/allowed.txt"]
```

### Other options

```toml
# Override target directory (default: $HOME)
target-directory = "/home/user"
```

See `spec/main.md` for complete configuration reference.

## Template Data & Profiles

`dotrift_data.toml` (in source directory) provides variables for template evaluation:

```toml
[variable]
hostname = "laptop"
editor = "nvim"
work_email = "user@company.com"

[profile.work]
hostname = "workstation"
email = "user@company.com"

[profile.personal]
email = "user@personal.com"
```

**Resolution order:**
1. Base `[variable]` section
2. Active profiles in activation order (last activated wins on conflict)

**Managing profiles:**

```bash
dotrift profile activate work
dotrift profile activate personal  # Now has highest precedence
dotrift profile show               # See resolved variables
dotrift profile list               # See which are active
```

Profiles active in the database but missing from `dotrift_data.toml` are silently ignored.

## Template Syntax

Templates use `{{ }}` for interpolation, `{% %}` for statements, and `{# #}` for comments.

### Example

```
{# Conditional block #}
{% if eq(hostname, "work") %}
export EMAIL="{{ work_email }}"
{% else %}
export EMAIL="{{ personal_email }}"
{% end %}

{# Loop over list #}
{% for pkg in packages %}
alias {{ pkg }}="~/.local/bin/{{ pkg }}"
{% end %}

{# Function calls #}
export PATH="{{ join(":", home(), ".local/bin", "$PATH") }}"
```

### Key features

- **Literals:** Strings (`"..."`), integers, booleans, lists
- **Dot access:** `obj.field`, `list.0`
- **Functions:** `eq(a, b)`, `and(...)`, `join(...)`, etc.
- **Control flow:** `{% if %}`, `{% elif %}`, `{% else %}`, `{% for %}`
- **Whitespace control:** `{{-` / `-}}` trims spaces, `{{=` / `=}}` eats entire line

See `spec/templater.md` for complete syntax reference.

## TUI Pager

Interactive terminal UI invoked during conflict resolution (`[d]iff` prompt). Three modes:

- **View** — Single file viewer with scrolling (used when file blocks directory creation)
- **Diff** — Side-by-side line diff with scroll sync (used for file conflicts)
- **Explorer** — Directory browser with file preview (used when directory blocks file creation)

**Navigation:** Vim-style keys (`j`/`k`, `Ctrl+D`/`Ctrl+U`, `g`/`G`), arrow keys, Page Up/Down. Press `h` for help.

The pager automatically selects the appropriate mode based on the paths involved.
