# Test data

Conventions for the data tests create: any file or directory name, any file
content (including template source files), and the variable names, values, and
profile names in `dotrift_data.toml`. These are defaults, not hard rules — when
the behavior under test requires otherwise, the agent decides.

## Principles

- Test data is boring and unopinionated. Zero personality: a file is `file`,
  a directory is `dir`. Avoid realistic-specific names like `.vimrc` or
  `.gitconfig`.
- Variation comes from a numeric suffix: `file1`, `file2`.
- Numbers and timestamps start at 1 and increment: `1`, `2`, ...
- File content is generic: `content1`, `content2`. Template source files are
  `{ str }` style.
- `dotrift_data.toml` variables mirror their values: `str = "str"`, `num = 1`.
  Profiles are `profile1`, `profile2`.
- Names inside content that map to filesystem entries (portal keys,
  destinations, ignore patterns, symlink targets, state paths) use the same
  generic naming and must match the filesystem entries the test creates.
