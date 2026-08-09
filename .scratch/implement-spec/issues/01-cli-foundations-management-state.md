# 01 — CLI Foundations and Management State

**What to build:** Global CLI path resolution, the state database, the state lock, managed checks, and the `status` command work for users.

**Blocked by:** None — can start immediately

**Status:** ready-for-agent

- [x] Global source and target options resolve according to precedence and path rules.
- [x] The state database stores managed paths and active-profile selectors at the specified location.
- [x] The state lock serializes state mutations and fails fast on contention.
- [x] Managed checks distinguish managed and unmanaged target paths using kind and fingerprint.
- [x] `status` reports records in target-path order without reading control files or acquiring the lock.
