---
name: rust-miette-crate
description: "Write readable Rust error messages with the `miette` crate — carry the message as the thiserror `#[error]` fragment, chain context with `.wrap_err()`, position notes with `#[label]` over `#[source_code]`, and advice as `help`. Use when writing, editing, or reviewing miette diagnostics, error types, or error-handling code in Rust."
---

# miette

`miette` turns errors into fancy diagnostics: an error message with optional
positioned notes drawn into a source snippet, separate help text, an error
code, a clickable URL, a severity, and a cause chain. It composes with
`thiserror`: keep the `Error` derive and add `Diagnostic` on top.

This skill catalogs the miette API so you know which construct produces which
part of a rendered report.

A rendered diagnostic has these parts, each fed by one API piece:

| report part | miette API |
| --- | --- |
| the message | thiserror `#[error("...")]` fragment |
| chained context | `.wrap_err("...")` (`WrapErr`) |
| positioned note in the source | `#[label("...")]` + `#[source_code]` |
| plain advice below the report | `#[help]` or `#[diagnostic(help(...))]` |
| machine-readable code | `#[diagnostic(code(pkg::name))]` |
| clickable link | `#[diagnostic(url(...))]` or `url(docsrs)` |
| severity (warning/advice) | `#[diagnostic(severity(Warning))]` |
| grouped sub-errors | `#[related]` |
| renders at top level to stderr | `main() -> miette::Result<()>` |

## Setup

```toml
[dependencies]
thiserror = "2"
```

A **binary** enables the `fancy` feature for graphical output:

```toml
[dependencies]
miette = { version = "7", features = ["fancy"] }
```

A **library** adds plain `miette` — `fancy` pulls heavy rendering dependencies
and belongs only in the top-level crate that prints. The `derive` feature (the
only default) is all a library needs. Optional features: `serde`
(`MietteDiagnostic` round-trips), `syntect-highlighter` (syntax highlighting,
enable from the binary side).

## Library vs application

Libraries define concrete error types and return them; applications wrap
anything into the boxed `Report`.

**Library** — `#[derive(Error, Debug, Diagnostic)]` on a `thiserror` error,
return the concrete type from fallible APIs. Wrap external errors with
`#[from]`; wrap a nested `Diagnostic` so its labels survive with
`#[error(transparent)]` + `#[diagnostic(transparent)]`. Never return
`miette::Result` or `Report` from a library — consumers that want labels can
convert on their side.

```rust
use miette::{Diagnostic, SourceSpan};
use thiserror::Error;

#[derive(Error, Debug, Diagnostic)]
#[error("missing `name` field")]
#[diagnostic(code(config::missing_name))]
pub struct MissingName {
    #[source_code]
    pub src: miette::NamedSource<String>,
    #[label("add `name` where the record is built")]
    pub at: SourceSpan,
}
```

**Application** — use `miette::Result<T>` (aliased to `Result<T, Report>`),
convert foreign errors with `.into_diagnostic()`, and build ad-hoc errors with
`miette!`, `bail!`, and `ensure!`.

```rust
use miette::{IntoDiagnostic, Result, WrapErr};

fn load(version: &str) -> Result<()> {
    version
        .parse::<semver::Version>()
        .into_diagnostic()
        .wrap_err(format!("cannot parse version `{version}`"))?;
    Ok(())
}
```

## The `Diagnostic` derive

All miette-specific attributes live on the `#[derive(Diagnostic)]` type, which
sits next to `#[derive(Error)]`:

### `#[error("...")]` — the message

The `#[error]` attribute (from `thiserror`) is the top-line message. Keep it a
single lowercase fragment describing what is wrong; the fix belongs on `help`,
not here. It supports `{field}` format arguments.

### `#[source_code]` and `#[label]` — positioned notes

A located error needs source plus a marker into it:

1. **`#[source_code]`** — one field holding the source the labels point into,
   any type implementing `SourceCode`. `String`, a borrowed `&str`, and
   `NamedSource::new("file.rs", text)` when a name should display cover the
   normal cases; a custom source (lazy, huge, or spanning multiple files) is in
   `references/dynamic-diagnostics-and-handlers.md`.
2. **`#[label("text")]`** — one or more fields of a span type; the label's
   text is a positioned note drawn under the range in the source snippet.

```rust
#[derive(Error, Debug, Diagnostic)]
#[error("unexpected token `{token}`")]
#[diagnostic(code(parser::unexpected_token))]
pub struct UnexpectedToken {
    token: String,
    #[source_code]
    src: String,
    #[label("parsed up to here")]
    at: usize,
    #[label("expected one of these")]
    expected: Option<(usize, usize)>,
}
```

`#[label]` accepts anything `Into<SourceSpan>`: `(offset, len)`, a `usize`
(offset, zero-length), `Range<usize>`, `RangeInclusive<usize>`, `SourceSpan`,
or `Option` of those for a label that may not apply. All offsets are **byte**
offsets into the source, not character or line positions.

One label per field (`#[label(primary, "...")]` marks the primary, at most
once per error; `#[label(collection, "...")]` takes any iterable of
`Into<SourceSpan>` when the count is unknown — details in
`references/dynamic-diagnostics-and-handlers.md`).

### `#[help]` — advice

Plain advice below the report rides on the `help` field, never in the
message. Static per error type:

```rust
#[derive(Error, Debug, Diagnostic)]
#[error("missing `name` field")]
#[diagnostic(help("add `name` where the record is built"))]
pub struct MissingName { /* ... */ }
```

Dynamic per instance — a field tagged `#[help]` (`String` or `Option<String>`):

```rust
#[error("cannot open `{path}`")]
#[diagnostic()]
pub struct CannotOpen {
    path: String,
    #[help]
    advice: Option<String>,
}
```

The static form takes a plain string; per-instance dynamic text goes on the
`#[help]` field. (Format arguments are not applied to `help`.)

### `#[diagnostic]` — code, url, severity

- `#[diagnostic(code(pkg::error_name))]` — a stable machine-readable code
  rendered with the report.
- `#[diagnostic(url(...))]` — makes the code a clickable link. `url(docsrs)`
  links to the error type's own docs.rs page; a format string to an arbitrary
  URL: `url("https://docs.example/errors#{}", code)`.
- `#[diagnostic(severity(Warning))]` — `Severity::Advice`, `Severity::Warning`,
  or `Severity::Error` (the default); non-`Error` severities render distinctly.

### `#[related]` — multiple errors

Tag an `IntoIter` field with `#[related]` to render sub-errors together under
one diagnostic — useful for validation that collects every failure instead of
stopping at the first. Each sub-error may carry its own `#[label]`; all draw
against the parent's `#[source_code]`.

```rust
#[derive(Error, Debug, Diagnostic)]
#[error("config has invalid fields")]
pub struct ConfigErrors {
    #[source_code]
    src: String,
    #[related]
    inner: Vec<ConfigError>,
}
```

## Runtime / app-side API

- `miette!("msg")` — build a `Report` from a message (like `anyhow!`); also
  accepts the same `severity`/`code`/`help`/`labels`/`url` keys as `diagnostic!`.
- `bail!("msg")` — return early with a `Report`: `return Err(miette!("msg").into())`.
- `ensure!(cond, "msg")` — bail unless the condition holds.
- `diagnostic!(...)` — like `miette!` but yields a `MietteDiagnostic`, a plain
  struct you can inspect, modify, serde-round-trip, and `.into()` a `Report`.
  Accepts `code`, `help`, `labels`, `url`, `severity` keys plus the message.
- `Err(my_error)?` — a `Diagnostic` error converts into `Report` via `From`.
- `.into_diagnostic()` — wrap any `std::error::Error` into a `Report`.
- `.wrap_err("context")` — `WrapErr`/`Context`, chained context.
- `.with_source_code(src)` — attach source to a `Report` at the place the
  source is known (e.g. `main` after a file is read).
- `LabeledSpan::at(range, label)` / `LabeledSpan::at_offset(offset, label)` /
  `LabeledSpan::new(Option<String>, offset, len)` — runtime-built labels, byte
  offsets into the source passed to `.with_source_code(...)`.

## Rendering

The fancy report renders once at the top-level handler: `main` returns
`miette::Result<()>` and the runtime prints the returned `Err` to stderr,
falling back to the narratable plain-text printer when stderr is not a
graphical terminal (off-TTY or `NO_COLOR`). `{:?}` on a `Report` renders the
same diagnostic anywhere; `{}` prints only the top message — prefer `{:?}`
where you print a report.

## Pointers

- Runtime-built diagnostics, delayed source, multiple related errors, custom
  handlers, and a `#[source_code]` that is not plain owned text (lazy, byte,
  or multi-file sources): read `references/dynamic-diagnostics-and-handlers.md`.
- API: https://docs.rs/miette/latest/miette/
