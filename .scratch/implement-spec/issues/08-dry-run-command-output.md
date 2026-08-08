# 08 — Dry Run and Command Output

**What to build:** Apply provides dry-run reporting, summaries, verbose and quiet output, flag validation, exit statuses, and complete concurrency behavior for the command surface.

**Blocked by:** 04 — Basic Apply Reconciliation; 05 — Copy, Template, and File Modes; 06 — Obstruction Resolution; 07 — Cleanup and Pruning

**Status:** ready-for-agent

- [ ] Dry-run performs preflight and reports intended actions without prompting, rendering deployed templates, or changing disk/state.
- [ ] Real-run summaries and verbose lines report deployment, replacement, skip, removal, and prune counts accurately.
- [ ] Quiet and verbose options, dry-run combinations, and other usage errors match the specification.
- [ ] Hard failures suppress summaries while completed-but-unsuccessful runs report their results.
- [ ] Apply lock contention and command exit statuses follow the specified behavior.
