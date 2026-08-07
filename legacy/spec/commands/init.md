# `init`

Initializes the source directory with a default `dotrift.toml`.

**Usage:** `dotrift init`

---

## Execution Pipeline

1. **Resolve Source Directory:** Use the source directory determined by global options (see spec/core.md section "Global CLI Structure").
2. **Check Existing Config:** Error if `dotrift.toml` already exists in the source directory.
3. **Create Config:** Write a default `dotrift.toml` to the source directory, creating parent directories as needed. The format of the written file is specified elsewhere (see spec/dotrift-toml.md section "Root Keys").