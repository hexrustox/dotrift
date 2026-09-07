# ADR-0018: A per-user global config file

dotrift's only configuration lived in the per-source control files, so
user-level preferences had no home: the pager could be chosen only through
environment variables, and behavior toggles had nowhere to persist. dotrift
now reads an optional per-user TOML file at
`$XDG_CONFIG_HOME/dotrift/config.toml` (falling back to
`$HOME/.config/dotrift/config.toml`), mirroring the state database's XDG
location. Three choices are pinned alongside it. The file is plain TOML,
never template-evaluated: it configures dotrift itself rather than a
deployment, so there is no variable context to render it with — the same
reasoning that keeps `dotrift_data.toml` untemplated (ADR-0003). It is
strict — unknown keys, unknown sections, and wrong types are configuration
errors rather than ignored input — because the settings it carries authorize
destructive behavior, and a typo that silently leaves `replace-identical`
unapplied is precisely the failure mode strictness prevents; this matches
`dotrift.toml`'s unknown-key rejection. And `apply` reads it eagerly, after
acquiring the state lock and before the control files, so a broken global
config fails every run like a broken control file instead of surfacing only
when a setting is first consulted. Rejected alternatives: CLI flags
(per-invocation, cannot persist a preference), and per-source overrides
(these are per-user machine preferences, not per-dotfiles decisions; layering
them into each source tree would recreate the control-file complexity the
global file exists to avoid).
