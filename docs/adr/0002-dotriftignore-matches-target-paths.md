# ADR-0002: `.dotriftignore` patterns match target paths, not source paths

`dotrift.toml` deliberately has no `ignore` field, so exclusions moved to a
standalone plain-text `.dotriftignore` at the source-directory root. Its
gitignore patterns match the *target* path of each resolved portal entry —
applied after portal resolution and before collision validation — even though
the file itself sits in the source tree. The obvious alternative, matching
source-relative paths, was rejected: a source path can resolve to several
targets, and "don't deploy this" is a statement about what lands on the target
directory, not about what lives in the source. Matching targets keeps those
cases expressible and aligns ignore evaluation with the outcome users observe.
