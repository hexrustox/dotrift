# Symlink-of-symlink deployment semantics

When a source path in the source directory is itself a symbolic link, dotrift never follows it during discovery — the symlink *is* the file being managed. But the deploy type controls what target path gets created, and the three deploy types diverge for source symlinks:

- **`copy`** of a source symlink → the target path becomes a symlink pointing to the *same destination* as the source symlink (identity preservation: source→`/a/b`, target→`/a/b`).
- **`symlink`** of a source symlink → the target path becomes a symlink pointing to the *source symlink itself* (indirection back into the source directory: source→`/a/b`, target→source).
- **`tmpl`** → the source symlink chain is followed to its ultimate resolution, the resolved file's content is read as a template, rendered, and the output is written as a regular file at the target path (content resolution: source→`/a/b`, target reads `/a/b`, renders template, writes result).

The asymmetry is intentional. `copy` means "preserve what the user has on disk (a link to somewhere else)." `symlink` means "indirect target readers back to my source directory." `tmpl` means "render this file's content" — which requires following the link to reach actual bytes.

A simpler rule — always follow source symlinks during discovery — would lose the identity-preservation case (`copy` users with symlinks in their dotfiles expect those symlinks to come out the other side intact). A second simpler rule — never follow, always treat the source symlink as the file — would make `tmpl` unusable for the common case of "the template file is itself a symlink to its canonical version." The three-way rule is the smallest consistent set that satisfies all three deploy types' user intent.
