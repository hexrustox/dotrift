# 04 — Basic Apply Reconciliation

**What to build:** `dotrift apply` deploys symlink entries in deterministic order, creates required parent directories, replaces managed paths automatically, and records completed actions in management state.

**Blocked by:** 01 — CLI Foundations and Management State; 03 — Configuration and Desired Deployment

**Status:** ready-for-agent

- [ ] Apply acquires the state lock before reading control files and holds it through completion.
- [ ] Missing and managed symlink targets deploy according to the specified action order.
- [ ] Parent directories are created as needed and are never recorded as managed paths.
- [ ] State records mirror successful writes and removals, while skipped or failed entries retain prior records.
- [ ] Deterministic entry ordering, target-root handling, and runtime source failures match the specification.
