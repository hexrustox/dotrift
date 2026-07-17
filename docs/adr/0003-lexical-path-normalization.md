# Lexical path normalization — no `canonicalize`, no symlink resolution

Dotrift's correctness rests on comparing paths across three domains — the source directory (`source-dir`), the target directory (`target-dir`), and the database (absolute `target_path` primary keys). It must answer questions like "is this target path inside `source-dir`?" and "does this database entry still correspond to the file on disk at this absolute path?" A naive implementation would call `std::fs::canonicalize` to resolve every path to its one true filesystem identity.

All path handling in dotrift uses **lexical normalization only** (via the `normalize-path` crate's `NormalizePath::normalize`: resolves `.` and `..` components, drops trailing slashes, does not touch the filesystem, does not resolve symlinks). Specifically:

- Source directory, target directory, portal keys, rule keys, and database `source_path` / `target_path` values are all normalized lexically.
- Symlinks in the source directory are *intentionally* preserved as symlinks during discovery — normalization never follows them.
- The single exception is the `templater` command's `--file` vs `--output` input-output conflict check (`src/command/templater.rs:40`), which calls `canonicalize().ok()` because it is a one-shot safety check before any I/O — and even there, `.ok()` swallows resolution failures so `None == None` does not trigger a false positive.

`canonicalize` would break three dotrift invariants:

1. **Symlink-as-file identity** (see ADR-0002) — a symlink in the source directory *is* the file being managed; canonicalizing would silently follow it and store the resolved destination as `source_path`, losing the identity of the symlink.
2. **Database-stable primary keys** — `target_path` is the primary key of `managed_files`; if normalization resolved symlinks, the same on-disk file reachable through two symlink paths would either hash to two different keys or silently merge, and re-applying after a symlink target changes would corrupt the database.
3. **Determinism across runs** — lexical normalization is a pure function of the path string; `canonicalize` depends on the live state of the filesystem, so the same `dotrift.toml` could produce different target paths on different machines or after a symlink is repointed.
