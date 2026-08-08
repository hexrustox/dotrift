# ADR-0007: Management state records last-applied identity only

For each successfully deployed target path, the management state records the
source path, the target path, a fingerprint of the last-applied target state,
and whether dotrift created a file or a symlink. The fingerprint is the link
target for a symlink and a hash of the deployed bytes for a copy or template.
Directories are never recorded. State mirrors completed filesystem actions: a
successful removal removes the corresponding record, and a successful write
creates or updates it; skipped or failed entries keep their prior records. The
record establishes that dotrift previously created a path, and comparing the
current kind and fingerprint against it decides whether the path is still
managed — a mismatch, including a kind change, makes the path an obstruction
(ADR-0004). Permissions are not part of the comparison. The record is not a
history or a transaction log. Recording directories was rejected because
directory structure is implicit and reconstructed from portals on every run,
and tracking it would leave stale state whenever the config changes.

Obstruction resolution transitions follow the same principle. Skipping a
modified managed path retains its record, so the path stays an obstruction
until its kind and fingerprint match again. Replacing it with a file or symlink
removes the old record when the obstruction is removed and writes the new
record when the replacement deploys; replacing it with a directory removes the
obsolete record, because directories are never recorded and the record names a
file or symlink kind only. Replacing an untracked path creates no record for a
directory and records only the child files and symlinks actually deployed.
Reversing that direction, a directory obstruction replaced by a file or symlink
is deleted deepest-first, one entry at a time, and each completed deletion
removes that descendant's record — managed or modified — because those paths no
longer exist and the user's `replace` choice authorised their removal.
