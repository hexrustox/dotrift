# 06 — Obstruction Resolution

**What to build:** Apply detects unmanaged obstructions and lets users skip, inspect, replace, or replace all, including recursive subtree replacement and the required management-state transitions.

**Blocked by:** 04 — Basic Apply Reconciliation; 05 — Copy, Template, and File Modes

**Status:** ready-for-agent

- [ ] Obstruction prompts expose the required filesystem metadata and consume the prompt API choices.
- [ ] `skip`, `view detail`, `replace`, and `replace all` have the specified effects on execution and exit status.
- [ ] Detail views show valid file diffs and render template output only when requested.
- [ ] Parent obstructions, symlink traversal boundaries, special objects, and recursive directory replacement follow the specification.
- [ ] Deletion and replacement update state after each completed action, including partial failures.
