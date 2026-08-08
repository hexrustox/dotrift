# ADR-0013: `--clean-up` removes stale managed paths, superseding ADR-0008

ADR-0008 deferred removal semantics to a separate `--clean-up` design rather
than folding deletion into `apply`. `--clean-up` resolves that deferral: it
removes *stale paths* — managed paths in the target directory that are not part
of the desired deployment — but only while they remain *managed paths*.

Deletion is gated by the managed check. A stale path whose filesystem kind and
fingerprint still match its record is provably dotrift's own unmodified output,
so removing it silently is consistent with the automatic replacement of managed
paths (ADR-0004). A stale path that no longer matches is an obstruction: user
data may sit behind it, so it is never silently deleted. Instead its record is
relinquished — dropped while the file is left untouched — making it an ordinary
untracked path. Records whose targets no longer exist are purged as stale state
(ADR-0007: state mirrors completed filesystem actions).

Two rejected alternatives shaped the design. First, deleting modified stale
paths (or prompting for each deletion) was rejected: silent deletion of
user-modified data breaks the obstruction contract, and prompting would fire
for files dotrift verifiably owns. Second, keeping records for modified stale
paths was rejected: such a record could never be cleanable again, and with no
command to drop records it would persist as permanent noise on every future
clean-up.

Clean-up runs only after a fully successful deploy walk — a run that failed
mid-deployment is already in a half-updated state and should not stack an
unprompted deletion pass on top of it. `--prune-empty-dirs` removes the empty
parent directories left behind, walking upward while empty, never past the
target root, and never through a symlink parent. Directories are never
recorded, so pruning touches no state.
