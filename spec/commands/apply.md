# `dotrift apply`

Reconciles the target directory to the *desired deployment*: reads the control
files, resolves and validates the portal entries, then creates or updates each
target path according to its effective rule. `apply` is the deployment command;
stale entries are removed only under `--clean-up` (see
[`--clean-up`](#--clean-up)).

## Pipeline

1. Acquire the state lock (see `spec/core.md § State lock`).
2. Resolve the source directory and read the control files:
   - `dotrift.toml`, rendered as a template and parsed (see
     `spec/dotrift-toml.md § Rendering before parsing`, ADR-0001).
   - `dotrift_data.toml`, parsed as plain TOML (see
     `spec/dotrift-data-toml.md`).
   - `.dotriftignore`, parsed as plain text (see `spec/dotriftignore.md`).
3. Resolve the portal entries.
4. Apply the ignore file's filtering stage (see
   `spec/dotriftignore.md § Filtering stage`).
5. Validate collisions and structural conflicts.
6. Resolve rules and compute the desired deployment.

The target directory is determined by CLI `--target` over `target-directory`
in `dotrift.toml`, defaulting to the home directory (see
`spec/commands/global.md § Target directory precedence`).

## Preflight

Before any filesystem change, `apply` verifies:

- The control files are readable and well-formed, and the rendered
  `dotrift.toml` is valid (see `spec/dotrift-toml.md § Validation`): literal
  portal sources exist, no two portal resolutions produce the same target
  path (collision), no two desired target paths place one as an ancestor of
  the other (structural conflict), and every source path is a regular file or
  a symlink to a regular file (see `spec/dotrift-toml.md § Source symlink
  behavior`).
- The source and target roots satisfy the overlap rule: the source directory
  may lie inside the target directory, but the target directory may not equal
  the source directory or lie inside it. Both roots are canonicalized before
  they are compared (see `spec/commands/global.md § Path resolution`,
  ADR-0012).
- The target directory root: absent → created during execution when the
  desired deployment is non-empty; a symlink root that resolves to a
  directory is allowed and deployed through; present but not resolving to a
  directory → error before deployment. The target root is never treated as an
  ordinary replaceable obstruction.

Preflight does not render deployed templates (see
[Template rendering](#template-rendering)). No source snapshot is taken:
entries read source content at their execution turn and observe any changes an
earlier entry made, with no special handling beyond the normal runtime failure
behavior.

## Execution

Entries deploy in target-path order, comparing paths component-wise (see
`spec/dotrift-toml.md § Path rules`).

For each entry, `apply` compares the existing target to the desired state:

- **Missing target:** create parent directories as needed and deploy, even
  when an obsolete state record for the target remains. The record is
  replaced when the state step of the deploy action runs.
- **Managed path:** replace automatically. A path is managed only when
  dotrift created it and both its current filesystem kind and fingerprint
  match the recorded ones (see `spec/core.md § Managed check`). Replacement
  happens on every run, even when the current bytes already equal what would
  be deployed now (ADR-0014): there is no unchanged-skip branch, the rule's
  mode is re-applied, and an identical re-run rewrites every managed path.
- **Obstruction:** stop and prompt the user (see below). This covers
  untracked targets, previously managed targets whose kind or fingerprint no
  longer matches, and special filesystem objects.

Before obstruction handling, the entry's source path is re-checked to be a
regular file or a symlink to one. A source path that disappears or changes
type after preflight but before its deployment turn is an execution failure:
the run stops, completed changes are preserved, and that entry's state record
is unchanged.

### Deploy action

Each deploy action runs in a fixed, non-atomic order (ADR-0010):

1. Write the file bytes or create the symlink. A template entry is rendered
   as the target is written: the target file is created and the rendered
   bytes are streamed into it (see
   [Template rendering](#template-rendering)).
2. Update the management state for the target.
3. Apply the configured mode, if the effective deploy type is `copy` or
   `template`.

A failure at any step returns an error and exits. Completed steps remain: a
state-write failure leaves the filesystem change in place, and a
mode-application failure leaves the new content and state in place with the
mode not applied. A render or write failure removes the partial target file
best-effort; a removal that itself fails leaves a partial file, which the
next run surfaces as an ordinary untracked obstruction.

### Obstruction prompts

The prompt shows the source path and the obstructing path, each with its
absolute location and its kind, and reports that the obstructing path is
already present:

```
Cannot deploy {kind} {source} because {kind} {obstruction} is already present.
How would you like to proceed?
```

The kind is reported from the path's own non-following metadata, one of
`directory`, `file`, `symlink`, or `unknown`: a directory is `directory`, a
regular file is `file`, a symlink is `symlink`, and any other filesystem
object — a FIFO, socket, device, or otherwise unresolvable kind — is
`unknown`. Paths are shown prettified per the path display convention (see
`spec/commands/global.md § Output conventions`). No size, modification time,
link target, or entry count is shown.

An unmanaged obstruction offers, in this order:

- `skip` (hotkey `s`) — leave the target unchanged and continue with later
  entries.
- `view diff` (hotkey `v`) — show a content diff of the two files, then return
  to the prompt.
- `replace` (hotkey `r`) — remove the obstruction and deploy the entry.
- `replace all` (hotkey `a`) — latch for this run: every upcoming obstruction
  prompt defaults to `replace` without prompting.

`replace` may remove any filesystem object, including recursively deleting a
non-empty directory. One `replace` decision authorises removal of the entire
obstructing subtree — untracked files, clean managed files, modified managed
files, nested directories, and special objects alike. Removal never follows
symlinks: a symlink inside the subtree, or the obstruction itself when it is a
symlink, is unlinked as a link and whatever it points at is left untouched.
Deletion runs one entry at a time, deepest-first in component order, and stops
at the first error; the state record is deleted after each completed removal,
so state is updated as each deletion completes. There is no `abort` option.
Cancelling the prompt (the prompt's cancel keys, `Esc` and `Ctrl+C`) ends the
run immediately: the cancelled entry is left untouched, no further entries are
attempted, no summary is printed, and `--clean-up` does not run. State
reflects only the actions already completed.

The prompt choices are provided by the TUI/prompt API; `apply` consumes the
API's result and does not implement terminal detection or a non-interactive
fallback. The API itself is non-interactive-safe: when stdin is not a
terminal, the prompt returns its default immediately without rendering
(`tui/spec/prompt.md § Defaults`). `apply` configures no default, so the
first option — `skip` — is chosen automatically: a piped run completes with
skips (exit `2`), and the `replace all` latch can never engage without a TTY.

`view diff` is offered only when both paths resolve, after following
symlinks, to regular files; it then shows a content diff of the two files.
For a template entry, the rendered output is diffed against the target; a
render failure shows the error and exits. For mixed file/directory kinds,
symlinks resolving to directories, or special objects, `view diff` is omitted
because no further useful information can be shown.

The diff is produced by the external `diff -u` command, with the raw
(unprettified) target and source paths as the diff labels. A `diff` exit
status of 1 — differences found — is normal; exit status 2 fails the run;
failure to start `diff` fails the run. The diff is displayed through the pager
named by `$DOTRIFT_PAGER` when set, otherwise `$PAGER`, otherwise printed to
standard output. An empty or whitespace-only value counts as unset, and the
value is split on whitespace into a program and its arguments (so
`less -R` works). A `$DOTRIFT_PAGER` that cannot be started fails the run; a
`$PAGER` that cannot be started falls back to standard output. A pager's
non-zero exit status never fails the run.

### Parent directories

Missing parent directories are created as needed and never recorded as
managed; they receive whatever permissions the process umask yields. A symlink
parent component that resolves to a directory is traversed: the entry deploys
through the link, and the write lands wherever the link resolves — possibly
outside the target directory root or inside the source tree. Only the roots
are overlap-checked, never a resolved parent. A required parent component that
exists but is neither a directory nor a symlink resolving to one is an
unmanaged obstruction resolved with the normal prompt. Replacing a parent
obstruction may remove the subtree beneath it, except that a symlink
obstruction is removed as a link, leaving whatever it points at untouched
(see [Obstruction prompts](#obstruction-prompts)).

## Template rendering

Deployed template entries render as part of the write step of their deploy
action (ADR-0006), so a render error fails that entry mid-run, after earlier
entries may already have changed the filesystem. Because state mirrors
completed filesystem actions, a render failure after an obstruction was
already removed leaves the target absent with its state removed. A render
error fails the run: completed actions remain, and the entry's state is
whatever the completed actions established.

`dotrift.toml` is unaffected: it is rendered eagerly before parsing
(ADR-0001).

## Management state

`apply` records a managed path as part of a successful deploy action:

- source path
- target path
- fingerprint of the last-applied target state: the link target for a
  symlink, or a hash of the deployed bytes for a copy or template (see
  `spec/core.md § Fingerprint`)
- whether the entry deployed as a file or a symlink

Directories are never recorded. State mirrors completed filesystem actions: a
successful removal removes the corresponding record, and a successful write
creates or updates the record. Skipped or failed entries retain their prior
records. A record whose target no longer exists, and which is no longer part
of the desired deployment, persists until `--clean-up` relinquishes it or a
future deploy reuses the path; a run without `--clean-up` never removes
records for paths outside the desired deployment (ADR-0013). Comparing the
recorded kind and fingerprint against the current target decides whether the
path is still managed and can be auto-replaced; permissions are not part of
the comparison.

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

`apply` holds the exclusive state lock for its entire run — acquired before
reading the control files and held through preflight, prompts, filesystem
actions, state updates, and the summary — and releases it when the run
completes (see `spec/core.md § State lock`, ADR-0011). A concurrent `apply`
that cannot acquire the lock fails through the normal command error path
rather than interleaving operations.

## Failure behavior

A runtime failure (for example a source path disappearing after preflight)
stops the run. Completed filesystem actions and state updates are preserved.
`apply` does not roll back, retry, or re-plan (ADR-0005). State-write failures
follow the same per-action handling. External interruption does not: there is
no signal handling — SIGINT or SIGTERM kills the process at an arbitrary
point, and the next run sees whatever the last completed step left behind
(ADR-0015).

## Dry-run

`--dry-run` performs the full preflight and walks the desired deployment,
reporting for each entry what a real run would do — deploy a new target,
replace a clean managed path, or require a user choice for an obstruction —
without prompting or changing anything under the target directory. Template
entries are reported like copy entries, without rendering. Dry-run prints no
summary, and it conflicts with both `--quiet` and `--verbose`. A dry-run
acquires the state lock like a real run: it fails when another `apply` holds
the lock. Opening the state database creates the state artifacts it always
creates (see `spec/core.md § State database`); no other state or filesystem
change occurs.

Each entry prints one line:

```
<action> <target-path> [<deploy-type>[ <mode>]]
```

* The action is `deployed` for a missing target, `replaced` for a clean
  managed path, and `obstruction` for an unmanaged target or an obstructing
  parent — the cases a real run would deploy, replace, or prompt about. The
  action word is colored per the palette (`spec/commands/global.md § Output
  conventions`): `deployed` green, `replaced` cyan, `obstruction` yellow.
* The target path is displayed per the path display convention.
* The bracket suffix shows the entry's effective deploy type — `symlink`,
  `copy`, or `template` — plus the effective mode as a three-digit octal
  number when a rule sets one: `[symlink]`, `[copy 600]`, `[template 644]`.

Under `--clean-up`, dry-run additionally reports the stale-path removals the
real run would perform, one `removed <path>` line per removal, and one
`pruned <path>` line per directory `--prune-empty-dirs` would remove,
following the same walk the real run performs. Relinquished records (missing
or modified stale paths) are reported by neither the real run nor the
dry-run.

## Output

A real run prints the obstruction prompts as they occur, plus one summary line
when the run's walk completes. Per-path detail is never printed by default.

The summary counts `deployed N, replaced N, skipped N`; under `--clean-up` it
also counts `removed N, pruned N` — the clean-up counts are present whenever
`--clean-up` was requested, zero when clean-up did not run (for example
because skips made the run unsuccessful). The summary line is plain text,
never colored. There is no failure count — a hard failure stops the run
before any summary — and relinquished records are never counted.

An entry counts as `deployed` when its target was absent and no obstruction
was removed for it, `replaced` when its target existed before the action or an
obstruction was removed for it — including user-confirmed replaces of
untracked targets and parent-obstruction replacements where the target itself
was absent — and `skipped` when its obstruction prompt was declined with
`skip`.

`--verbose` adds one line per acted-upon path as the walk proceeds, in the
form `<action> <path>` where the action word is colored per the palette
(`deployed` green, `replaced` cyan, `skipped` dark grey) and the path is
displayed per the path display convention (`spec/commands/global.md § Output
conventions`); under `--clean-up` it also prints `removed` (red) and `pruned`
(magenta) lines, leaving the default clean-up silence intact otherwise.

`--quiet` suppresses the summary line. Prompts and errors are never
suppressed.

The flag combinations `--quiet` with `--verbose`, `--dry-run` with `--quiet`,
and `--dry-run` with `--verbose` are usage errors.

A hard failure — a deploy, removal, or prune error — stops the run before the
summary is printed. A completed-but-unsuccessful run (for example one with
obstruction skips) still prints the summary, reflecting the skips.

## `--clean-up`

`--clean-up` removes *stale paths*: managed paths in the target directory that
are not part of the desired deployment. It runs after a successful deploy walk
only — an unsuccessful run (see [Exit status](#exit-status): a skip, a
failure, or a failed removal or prune) leaves clean-up for the next
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
removed. Removal reaches through symlink parent components: a stale managed
file behind a symlink parent is unlinked. A stale path modified since the
last apply is an obstruction and is never silently deleted: the file is left
untouched and its record is relinquished, making it an ordinary untracked
path. A record whose target no longer exists is relinquished as well, with
nothing left on disk. Relinquishing is never reported and never fails the
run (see [Dry-run](#dry-run)).

`--clean-up` is silent by default: a real run prints nothing per removed path,
and its summary line still reports the removal and prune counts. `--verbose`
adds per-path `removed` and `pruned` lines (see [Output](#output)). A removal
that fails at the filesystem level fails the run through the normal error
path.

`--dry-run --clean-up` reports the removals and prunes the real run would
perform, without changing the filesystem.

### `--prune-empty-dirs`

`--prune-empty-dirs` may only be used together with `--clean-up`; using it
alone is an error. After each removal, the parent chain of the removed path is
walked upward while each directory is empty, removing it, and stopping at the
first non-empty directory. The target directory root is never pruned, and the
walk never crosses a symlink component: a symlink parent is never removed, and
directories behind one are never pruned, even when empty. Pruning runs in the
same post-deploy phase, immediately after each removal, so a directory that
still holds a deployed entry is non-empty and is left alone. Directories are
never recorded, so pruning touches no state.

## Exit status

`apply` exits with a process exit code from three values:

* **`0`** — success: the walk completed, no entry was skipped or failed, and
  (under `--clean-up`) no removal or prune failed.
* **`1`** — error or cancellation: preflight validation failed, a runtime
  failure stopped the run, or the user cancelled an obstruction prompt.
* **`2`** — completed with skips: the walk finished but at least one entry
  was skipped.

A completed `--dry-run` always exits `0`, whatever the desired deployment
contains: it deploys nothing, skips nothing, and prompts for nothing.

The command-level notion of an "unsuccessful" run covers both `1` and `2`:
under `--clean-up`, a removal or prune that fails at the filesystem level also
makes the run unsuccessful. Relinquishing a stale path never makes the run
unsuccessful.
