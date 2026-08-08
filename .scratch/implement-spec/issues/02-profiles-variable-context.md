# 02 — Profiles and Variable Context

**What to build:** Users can list, activate, deactivate, and show template profiles, with active selections persisted and resolved into the variable context.

**Blocked by:** 01 — CLI Foundations and Management State

**Status:** ready-for-agent

- [ ] `profile list` shows defined profiles in lexical order and marks active profiles.
- [ ] `profile activate` validates definitions, persists activation timestamps, and updates precedence when reactivated.
- [ ] `profile deactivate` removes active profiles, including stale definitions.
- [ ] `profile show` renders the resolved context with the specified overlay and canonical-value rules.
- [ ] Missing files, stale activations, unsupported values, and malformed data produce the specified results.
