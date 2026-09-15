# Test data

Conventions for the any data tests create.

## Principles

- Test data is boring and unopinionated. Zero personality: a file is `file1`,
  a directory is `dir1`, a nested directory is `dir1/sub1`. Avoid realistic
  names like `.vimrc` or `.gitconfig`, and bare letters like `a`, `a/b`, `f`.
- Variation comes from the numeric suffix: `file1`, `file2`, `dir1`, `sub1`.
  Numbers and timestamps start at 1 and increment per kind of entry.
- Every path component uses this scheme, including symlink entries (`link1`,
  `link2`). Role names like `real`, `dirlink`, `broken`, `nowhere`, `loop`,
  `keep`, `empty`, `ok` are not allowed; the meaning lives in the test-case
  name, not the filename.
- File content is generic: `content1`, `content2`. Template source files are
  `{{ str }}` style.
- `dotrift_data.toml` variables mirror their values: `str = "str"`, `num = 1`.
  Profiles are `profile1`, `profile2`.
- Names inside content that map to filesystem entries (portal keys,
  destinations, ignore patterns, symlink targets, state paths) use the same
  generic naming and must match the filesystem entries the test creates.
