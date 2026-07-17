# `templater`

The `templater` *subcommand* evaluates a dotrift template standalone and
writes the rendered output to stdout or a file. The template *engine* syntax
itself is specified in `spec/templater.md` — this file documents only the
command surface.

**Usage:** `dotrift templater [OPTIONS]`

**Template Source** (exactly one required):

* `-s, --string <TEMPLATE>`: Inline template string to evaluate.
* `-f, --file <PATH>`: Path to a template file on disk.

**Options:**

* `-o, --output <PATH>`: Write rendered output to the specified file instead of stdout. Parent directories are created if they do not exist.
* `-v, --var <KEY=VALUE>`: Set a template variable. Repeatable. Value is parsed as a TOML literal (string, integer, boolean, array, or inline table).
* `--no-data`: Do not load `dotrift_data.toml` or active profiles. Only `--var` variables are available.
* `--data-path <PATH>`: Explicit path to `dotrift_data.toml`. When omitted, resolves from the source directory (respects `-s, --source`).

---

## Errors

* **Missing template source:** Error if neither `--string` nor `--file` is provided.
* **Ambiguous template source:** Error if both `--string` and `--file` are provided.
* **Input-output conflict:** Error if `--file` and `--output` resolve to the same path. (This check is the single exception to the lexical-only path rule — see `spec/core.md` Path Normalization and ADR-0003.)
* **Mutually exclusive flags:** Error if both `--no-data` and `--data-path` are provided.
* **Template errors:** Parse and render errors are fatal, reported with source annotations.
* **`--var` parse errors:** Fatal.
* **DB errors:** Fatal.

---

## Execution Pipeline

1. **Resolve template source:** If `--string`, use the inline string directly. If `--file`, read the file from disk.
2. **Resolve variables** (unless `--no-data`): Load `dotrift_data.toml` from `--data-path` if provided, or from the source directory. Missing file is treated as empty (no variables from file). Query the database for active profiles. Merge in order: `[variable]` → active profiles (by Profile Resolution, `spec/dotrift-data-toml.md#profile-resolution`).
3. **Apply `--var` overrides:** Each `KEY=VALUE` argument sets a variable, Overwrites any conflicting key from step 2.
4. **Evaluate template:** Evaluate the template with the resolved variable context and the same built-in functions available to `apply`. Template syntax follows `spec/templater.md`.
5. **Output:** Write rendered content to stdout, or to the file specified by `--output` (creating parent directories as needed).
