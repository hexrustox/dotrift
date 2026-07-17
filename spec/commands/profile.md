# `profile`

Manages template profiles. The profile format and the resolution algorithm
are specified in `spec/dotrift-data-toml.md`; the `active_profiles` storage
schema is in `spec/core.md`.

**Usage:** `dotrift profile <SUBCOMMAND> [ARGS]`

**Subcommands:**

* `list`: Show all profiles from `dotrift_data.toml`, mark active ones.
* `activate <name>`: Activate a profile. Re-activating an already-active profile updates its timestamp to now (moves it to last in precedence).
* `deactivate <name>`: Deactivate a profile. Error if not active.
* `show`: Print the resolved variable context as a two-column key-value table.

---

## Errors

* **`dotrift_data.toml` parse failure:** Fatal. Applies to any subcommand that parses the file (`list`, `activate`, `show`).
* **`list` — no profiles:** Error if `dotrift_data.toml` is missing or has no `[profile]` entries.
* **`activate <name>` — undefined:** Error if `<name>` is not defined in `[profile]`.
* **`deactivate <name>` — not active:** Error if profile is not currently active.
* **DB errors:** Fatal.

---

## Execution Pipeline

**`list`:**

1. Parse `dotrift_data.toml`.
2. Query DB for active profiles.
3. Print each profile name. Active ones annotated with `(active)`.

**`activate <name>`:**

1. Parse `dotrift_data.toml`.
2. `INSERT OR REPLACE` into `active_profiles` (REPLACE updates `activated_at`).

**`deactivate <name>`:**

1. Delete from `active_profiles` where `name` = `<name>`.

**`show`:**

1. Parse `dotrift_data.toml`.
2. Query active profiles in activation order.
3. Merge variables per Profile Resolution (`spec/dotrift-data-toml.md#profile-resolution`): `[variable]` base, then each profile in activation order (last wins).
4. Print as a two-column table (`key` | `value`). If no variables and no active profiles, print nothing.
