# 07 — Cleanup and Pruning

**What to build:** `--clean-up` removes stale managed paths safely after a successful deployment walk, relinquishes records it cannot remove, and optionally prunes empty parent directories.

**Blocked by:** 04 — Basic Apply Reconciliation; 06 — Obstruction Resolution

**Status:** ready-for-agent

- [ ] Stale-path candidates are limited to records under the current target root and outside the desired deployment.
- [ ] Managed stale paths are removed, modified stale paths are left untouched and relinquished, and missing targets are relinquished.
- [ ] Cleanup runs only after a successful deployment walk and follows the specified failure behavior.
- [ ] `--prune-empty-dirs` requires cleanup, never prunes the target root, and does not traverse symlink parents.
- [ ] Cleanup output and summary counts match default, verbose, and dry-run behavior.
