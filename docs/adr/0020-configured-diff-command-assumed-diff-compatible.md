# ADR-0020: A configured diff command is assumed diff-compatible

`view diff` hardcoded the external invocation as `diff -u` with the compared
paths as `--label` values, so users of other diff tools — difftastic, delta,
`git diff --no-index` — could not change it. The global config now carries an
optional `[diff]` table (see `spec/global-config.md § [diff]`), and the
configured `command` replaces the built-in invocation outright: dotrift adds
no `-u` and no `--label` arguments. Two choices are pinned alongside it. The
compared files reach the command only through the `${target}` / `${source}`
placeholders (substituted textually into `args`, each element staying a
single argument), with the pair appended when args name neither — labels are
available as `${target-label}` / `${source-label}` because arbitrary tools do
not understand `diff`'s `--label` flag, and a fixed-flag injection would make
every non-`diff` tool unusable or force a wrapper script. And a configured
command inherits `diff`'s exit-status semantics — exit 0 (no differences) and
1 (differences found) are normal, exit 2 fails the run, as does a failure to
start — because a diff tool's status is meaningful exactly where `diff`'s is:
exit 2 is how a diff-family tool reports that it could not compare, and
tolerating any non-zero exit would swallow that failure as "no output",
turning a breakage into a silently empty view diff. Tools far outside the
diff family can still be used by wrapping them in a script that maps their
exit codes. Rejected alternatives: passing the built-in flags anyway (couples
the config to GNU diff's CLI and breaks other tools), falling back to the
built-in `diff -u` when the configured command fails to start (a failing
launch then silently shows a different output format than the user chose),
and treating every non-zero exit as failure (every real diff with
differences would fail the run).
