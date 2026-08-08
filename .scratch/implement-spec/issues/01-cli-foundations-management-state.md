# 01 — CLI Foundations and Management State

**What to build:** Global CLI path resolution, the state database, the state lock, managed checks, and the `status` command work for users.

**Blocked by:** None — can start immediately

**Status:** ready-for-agent

- [ ] Global source and target options resolve according to precedence and path rules.
- [ ] The state database stores managed paths and active-profile selectors at the specified location.
- [ ] The state lock serializes state mutations and fails fast on contention.
- [ ] Managed checks distinguish managed and unmanaged target paths using kind and fingerprint.
- [ ] `status` reports records in target-path order without reading control files or acquiring the lock.
