# ADR-0014: `apply` always replaces managed paths

`apply`'s per-entry decision has exactly three branches: a missing target is
deployed, a managed path is replaced, an obstruction is prompted (ADR-0004).
There is no fourth branch. A managed path — one whose current kind and
fingerprint still match its state record — is rewritten on every run, even when
its current bytes already equal what would be deployed now.

The fingerprint exists to answer "did dotrift write this, and has anything
touched it since?", not "is this what the current configuration would produce?"
Managed-ness is a trust judgment about authorship; content difference is not
part of the question. Checking freshness would require rendering every template
and hashing every source file *before* deciding to act, which preflight
deliberately does not do (ADR-0006): entries read source content at their
execution turn.

Three consequences are accepted. First, an identical re-run rewrites every
managed path: mtimes churn, and the summary reports `replaced N` for all N.
Second, the rule's `mode` is re-applied on every deploy, so a permission drifted
by hand self-heals on the next run even when the bytes are unchanged. Third,
`--dry-run` is a preview of what `apply` will *do*, not what will *change*: it
reports `replaced` for every clean managed path, on every run, and cannot be
used as a change detector.

The rejected alternative was an "unchanged, skip" branch comparing the current
on-disk content against a fresh render or copy. It would have made dry-run a
change detector and eliminated mtime churn, at the cost of a fourth summary
outcome, a second fingerprint comparison per entry, and eager template
rendering that ADR-0006's lazy model forbids.
