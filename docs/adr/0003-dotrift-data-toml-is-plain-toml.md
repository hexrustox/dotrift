# ADR-0003: `dotrift_data.toml` is plain TOML, not template-rendered

`dotrift_data.toml` is parsed directly as TOML and is never evaluated as a
template. This is the deliberate counterpart to ADR-0001, which renders
`dotrift.toml`: the data file is the *source* of the variable context, so
rendering it would require a bootstrap context and make profile loading
circular. The alternative — templating it against an empty context or a
predefined set of variables — would introduce ordering ambiguity and a second,
special-purpose variable source for no benefit, since environment expansion is
already unsupported.