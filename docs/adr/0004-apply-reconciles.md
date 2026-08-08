# ADR-0004: `apply` reconciles — replacing managed paths and prompting on unmanaged obstructions

`dotrift apply` brings the target directory toward the *desired deployment*
rather than blindly writing according to the config. A target path that dotrift
created and whose current filesystem kind and fingerprint still match the
record — a *managed path* — is replaced automatically. Everything else that
occupies a target path is an *obstruction* and is never overwritten silently:
untracked paths, previously managed paths whose kind or fingerprint no longer
match, special filesystem objects, and symlink parent components. Obstructions
stop `apply` and prompt the user to `skip`, view details, `replace`, or
`replace all`; `replace all` latches the run so every upcoming prompt defaults
to `replace`. One `replace` decision authorises removal of the entire
obstructing subtree, and `replace` may remove any filesystem object, including
recursively deleting a non-empty directory. The prompt has no `abort` choice:
`skip` is the sole non-destructive way to decline, and choosing `skip` makes
the overall run unsuccessful. The rejected
alternative was a blind overwrite model where the config wins over whatever
exists on disk; that would let a misconfigured portal silently destroy user
files.
