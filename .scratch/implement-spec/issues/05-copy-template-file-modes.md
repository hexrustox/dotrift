# 05 — Copy, Template, and File Modes

**What to build:** Apply supports copy and template deploy types, profile-aware template rendering, content fingerprints, and configured file modes.

**Blocked by:** 02 — Profiles and Variable Context; 03 — Configuration and Desired Deployment; 04 — Basic Apply Reconciliation

**Status:** ready-for-agent

- [ ] Copy deployments write source bytes and record the resulting file fingerprint.
- [ ] Template deployments use the resolved variable context and render immediately before writing.
- [ ] Template rendering is skipped during preflight and dry-run.
- [ ] Modes apply only to copy and template deployments and follow the specified non-atomic action order.
- [ ] Render, write, state, and mode failures preserve completed filesystem and state changes.
