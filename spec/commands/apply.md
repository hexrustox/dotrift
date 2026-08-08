# `dotrift apply`

Reconciles the target directory to the *desired deployment*: reads the control
files, resolves and validates the portal entries, then creates or updates each
target path according to its effective rule. `apply` is the deployment command;
stale entries are removed only under `--clean-up` (see
[`--clean-up`](#--clean-up)).

## Pipeline

1. Acquire the state lock (see [Concurrency](#concurrency)).
2. Resolve the source directory and read the control files:
   - `dotrift.toml`, rendered as a template and parsed (see
     [`../dotrift-toml.md`](../dotrift-toml.md), ADR-0001).
   - `dotrift_data.toml`, parsed as plain TOML (see
     [`../dotrift-data-toml.md`](../dotrift-data-toml.md)).
   - `.dotriftignore`, parsed as plain text (see
     [`../dotriftignore.md`](../dotriftignore.md)).
3. Resolve the portal entries.
4. Apply the ignore file's filtering stage.
5. Validate collisions and structural conflicts.
6. Resolve rules and compute the desired deployment.

The target directory is determined by CLI `--target` over `target-directory`
in `dotrift.toml`, defaulting to `$HOME`.

## Preflight

Before any filesystem change, `apply` verifies:

- The control files are readable and well-formed, and the rendered
  `dotrift.toml` is valid (see `dotrift-toml.md` Validation).
- Every literal portal source path exists. A literal naming a missing source
  path is a configuration error. Glob portals that match zero source paths are
  valid no-ops.
- No two portal resolutions produce the same target path (collision), and no
  two desired target paths place one as an ancestor of the other (structural
  conflict).
- The source and target roots satisfy the overlap rule: the source directory
  may lie inside the target directory, but the target directory may not equal
  the source directory or lie inside it.
- Every source path is a regular file or a symlink to a regular file. Special
  source filesystem objects (FIFOs, sockets, device files, and so on) are
  configuration errors for every deploy type.
- The target directory root: absent → created during execution when the
  desired deployment is non-empty; present but not a directory → error before
  deployment. The target root is never treated as an ordinary replaceable
  obstruction.
- A copy or template entry whose source is a symlink that does not resolve to a
  regular file fails preflight.

Preflight does not render deployed templates (see
[Template rendering](#template-rendering)). No source snapshot is taken:
entries read source content at their execution turn and observe any changes an
earlier entry made, with no special handling beyond the normal runtime failure
behavior.

## Execution

Entries deploy in lexicographic order of their target-relative path.

For each entry, `apply` compares the existing target to the desired state:

- **Missing target:** create parent directories as needed and deploy, even when
  an obsolete state record for the target remains. The record is replaced when
  the state step of the deploy action runs.
- **Managed path:** replace automatically. A path is managed only when dotrift
  created it and both its current filesystem kind and fingerprint match the
  recorded ones.
- **Obstruction:** stop and prompt the user (see below). This covers untracked
  targets, previously managed targets whose kind or fingerprint no longer
  matches, and special filesystem objects.

A source path that disappears or changes type after preflight but before its
deployment turn is an execution failure: the run stops, completed changes are
preserved, and that entry's state record is unchanged.

### Deploy action

Each deploy action runs in a fixed, non-atomic order:

1. Write the file bytes or create the symlink.
2. Update the management state for the target.
3. Apply the configured mode, if the effective deploy type is `copy` or
   `template`.

A failure at any step returns an error and exits. Completed steps remain: a
state-write failure leaves the filesystem change in place, and a
mode-application failure leaves the new content and state in place with the
mode not applied.

### Obstruction prompts

The prompt always shows the metadata below for each path involved:

- existence and filesystem kind: regular file, directory, symlink, or other
- absolute path
- regular file: byte size and last-modified date
- symlink: link target
- directory: number of entries
- other: no kind-specific metadata

An unmanaged obstruction offers:

- `skip` — leave the target unchanged and continue with later entries.
- `view detail` — show a content diff of the two files, then return to the
  prompt.
- `replace` — remove the obstruction and deploy the entry.
- `replace all` — latch for this run: every upcoming obstruction prompt
  defaults to `replace` without prompting.

`replace` may remove any filesystem object, including recursively deleting a
non-empty directory. One `replace` decision authorises removal of the entire
obstructing subtree — untracked files, clean managed files, modified managed
files, nested directories, and special objects alike. Deletion runs one entry
at a time, deepest-first, and stops at the first error; state is updated after
each completed deletion. There is no `abort` option; an external interrupt may
terminate the process under normal OS signal handling.

The prompt choices are provided by the TUI/prompt API; `apply` consumes the
API's result and does not implement terminal detection or a non-interactive
fallback.

`view detail` is offered only when both paths resolve, after following
symlinks, to regular files; it then shows a content diff of the two files.
For a template entry, the rendered output is diffed against the target; a
render failure shows the error and exits. For mixed
file/directory kinds, symlinks resolving to directories, or special objects,
`view detail` is omitted because no further useful information can be shown.

### Parent directories

Missing parent directories are created as needed and never recorded as
managed. A required parent component that exists as a non-directory is an
unmanaged obstruction resolved with the normal prompt. A symlink parent
component is always an obstruction, even when it resolves to a directory —
symlinks are never followed for traversal. Replacing a parent obstruction may
remove the subtree beneath it.

## Template rendering

Deployed template entries render immediately before the write step of their
deploy action, so a render error fails before the target is written. Because
state mirrors completed filesystem actions, a render failure after an
obstruction was already removed leaves the target absent with its state
removed. A render error fails the run: completed actions remain, and the
entry's state is whatever the completed actions established.

`dotrift.toml` is unaffected: it is rendered eagerly before parsing
(ADR-0001).

## Management state

`apply` records a managed path as part of a successful deploy action:

- source path
- target path
- fingerprint of the last-applied target state: the link target for a
  symlink, or a hash of the deployed bytes for a copy or template
- whether the entry deployed as a file or a symlink

Directories are never recorded. State mirrors completed filesystem actions: a
successful removal removes the corresponding record, and a successful write
creates or updates the record. Skipped or failed entries retain their prior
records. Comparing the recorded kind and fingerprint against the current
target decides whether the path is still managed and can be auto-replaced;
permissions are not part of the comparison.

### State transitions on obstruction resolution

- **Untracked obstruction:** no record exists, so `skip` creates nothing and
  `replace` records only after the replacement writes. Replacing with a
  directory records nothing itself; its child files and symlinks are recorded
  as they deploy.
- **Modified previously-managed obstruction:** `skip` retains the old record
  unchanged. A successful `replace` with a file or symlink removes the old
  record when the obstruction is removed and writes the new record when the
  replacement deploys. A successful `replace` that creates a directory removes
  the obsolete record — directories are never recorded — and child entries are
  recorded as they deploy.
- **Directory obstruction replaced by a file or symlink:** descendants are
  deleted deepest-first, one at a time; each completed deletion removes that
  descendant's record, clean and modified alike. After the directory itself is
  removed, the replacement file or symlink is recorded when it deploys.
- **Replacement failure:** state reflects only the completed actions. If the
  obstruction was removed but the replacement failed to render or write, the
  target is absent and its record is gone. If a directory deletion stopped part
  way, the completed deletions and their record removals stand.

## Concurrency

`apply` holds the exclusive state lock for its entire lifecycle — acquired
before reading the control files and held through preflight, prompts,
filesystem actions, state updates, and exit. A concurrent `apply` that cannot
acquire the lock fails through the normal command error path rather than
interleaving operations.

## Failure behavior

A runtime failure (for example a source path disappearing after preflight)
stops the run. Completed filesystem actions and state updates are preserved.
`apply` does not roll back, retry, or re-plan. External interruption and
state-write failures follow the same per-action handling.

## Dry-run

`--dry-run` performs the full preflight and walks the desired deployment,
reporting for each entry what a real run would do — deploy a new target,
replace a clean managed path, or require a user choice for an obstruction —
without prompting or changing the filesystem. Template entries are reported
like copy entries, without rendering. Dry-run prints no summary, and it
conflicts with both `--quiet` and `--verbose`.

## Output

A real run prints the obstruction prompts as they occur, plus one summary line
when the run's walk completes. Per-path detail is never printed by default.

The summary counts `deployed N, replaced N, skipped N`; under `--clean-up` it
also counts `removed N, pruned N`. There is no failure count — a hard failure
stops the run before any summary — and relinquished records are never counted.

`--verbose` adds one line per acted-upon path as the walk proceeds (`deployed`,
`replaced`, `skipped`); under `--clean-up` it also prints `removed` and `pruned`
lines, leaving the default clean-up silence intact otherwise.

`--quiet` suppresses the summary line. Prompts and errors are never suppressed.

The flag combinations `--quiet` with `--verbose`, `--dry-run` with `--quiet`,
and `--dry-run` with `--verbose` are usage errors.

A hard failure — a deploy, removal, or prune error — stops the run before the
summary is printed. A completed-but-unsuccessful run (for example one with
obstruction skips) still prints the summary, reflecting the skips.

## `--clean-up`

`--clean-up` removes *stale paths*: managed paths in the target directory that
are not part of the desired deployment. It runs after a successful deploy walk
only — a run that failed during deployment leaves clean-up for the next
invocation.

### Candidates

A stale path is a *state record* whose target path lies under the current
target directory root and is not part of the desired deployment. Records from
other deployments, outside the current target root, are never candidates.
Paths excluded by the ignore file are not in the desired deployment and are
therefore stale: adding an ignore pattern makes the previously deployed path a
removal candidate, while a negated pattern that re-includes it removes it from
the candidate set.

Only a stale path whose managed check passes — still a *managed path* — is
removed. A stale path modified since the last apply is an obstruction and is
never silently deleted: the file is left untouched and its record is
relinquished, making it an ordinary untracked path. A record whose target no
longer exists is relinquished as well, with nothing left on disk. Relinquishing
neither fails the run nor is reported outside a dry-run.

`--clean-up` is silent by default: a real run prints nothing per removed path,
and its summary line still reports the removal and prune counts. `--verbose`
adds per-path `removed` and `pruned` lines (see [Output](#output)). A removal
that fails at the filesystem level fails the run through the normal error
path.

`--dry-run --clean-up` reports the removals, relinquishments, and prunes the
real run would perform, without changing the filesystem.

### `--prune-empty-dirs`

`--prune-empty-dirs` may only be used together with `--clean-up`; using it
alone is an error. After each removal, the parent chain of the removed path is
walked upward while each directory is empty, removing it, and stopping at the
first non-empty directory. The target directory root is never pruned, and a
symlink parent component is never traversed or pruned. Pruning runs after
removals in the same post-deploy phase, so a directory that still holds a
deployed entry is non-empty and is left alone. Directories are never recorded,
so pruning touches no state.

## Exit status

The run is unsuccessful if preflight validation failed, or if any entry was
skipped or failed; otherwise it succeeds. Under `--clean-up`, a removal or
prune that fails at the filesystem level also makes the run unsuccessful.
Relinquishing a stale path never makes the run unsuccessful.
