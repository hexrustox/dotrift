# ADR-0019: Identical obstructions may be replaced without prompting

ADR-0004 made every obstruction prompt, and the safety that buys is real —
but the prompt is pure ceremony when the obstructing file's content already
equals what would be deployed: untracked files left over from a pre-dotrift
layout, touched-but-unchanged previously-managed files, symlinks already
pointing at the right source. The prompt also actively blocks automation:
non-interactive runs take the prompt's `skip` default, so an `apply` over an
already-correct-but-unmanaged tree could never complete successfully when
piped. With the global config's `replace-identical` enabled (ADR-0018), such
an *identical obstruction* is now replaced without prompting, before the
prompt is raised. The safety invariant survives because identity is
established by comparing content — file fingerprint equality for `copy` and
`template` deploys, link-target equality for `symlink` deploys — not by
config declaration: a misconfigured portal still cannot destroy divergent
content, only bytes identical to what replaces them, which carries no
information loss. The scope is deliberately narrow: only the entry's own
target path qualifies (never parent obstructions, directories, or special
filesystem objects), any read or render failure during the check means "not
identical" and prompts as before, file mode is ignored because the
replacement re-applies the rule's mode, and the `replace all` latch subsumes
the check entirely. Rejected alternative: a config option to auto-replace
*all* obstructions — that would reintroduce the blind-overwrite model
ADR-0004 exists to prevent.
