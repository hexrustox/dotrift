---
name: rust-miette-crate
description: "Implement the writing-error-messages rules in Rust with the `miette` crate — carry the message as the thiserror `#[error]` fragment, chain context with `.wrap_err()`, the positioned note as a `#[label]` over a `#[source_code]`, and advice as `help`. Define `#[derive(Error, Debug, Diagnostic)]` error types, raise and propagate errors with `miette::Result`, `.into_diagnostic()`, `miette!`, `bail!`, `ensure!`, and render fancy diagnostics by returning `miette::Result` from `main`. Use when writing, editing, or reviewing miette diagnostics, error types, or error-handling code in Rust."
---

# miette

`miette` turns errors into fancy diagnostics: an error message with optional
positioned notes drawn into a source snippet, separate help text, an error
code, and a cause chain. It is the Rust rendering backend for the
`writing-error-messages` rules — the wording lives there, the carrier lives
here.

## The rules map onto miette constructs

Run `writing-error-messages` over every message; place each piece on the
structure that renders it:

| writing-error-messages | miette |
| --- | --- |
| the message (rules 1–3) | thiserror `#[error("...")]` fragment |
| chained context (rule 3) | `.wrap_err("...")` (`WrapErr`) |
| positioned note (rule 5) | `#[label("...")]` + `#[source_code]` |
| advice, plain note, internal-error note (rules 4, 5, 7) | `help` |
| renders at top-level handler, stderr (rule 6) | `main() -> miette::Result<()>` |

Never concatenate advice into the message (rule 4): the label and the `help`
text are the message's separate pieces.

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
only default) is all a library needs.

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

## The message — `#[error("...")]`

The `#[error]` attribute is the message. Write it exactly per
`writing-error-messages`: one lowercase fragment, no period, backticked tokens,
what is wrong rather than the fix. `#[error("try parsing a valid version")]`
prescribes — the fix belongs on `help`, not here.

## The positioned note — `#[label]`

A located error (rule 5) needs source plus a marker into it:

1. **`#[source_code]`** — one field holding the source the labels point into,
   any type implementing `SourceCode`. `String`, a borrowed `&str`, and
   `NamedSource::new("file.rs", text)` when a name should display cover the
   normal cases; a custom source (lazy, huge, or spanning multiple files) is in
   `references/dynamic-diagnostics-and-handlers.md`.
2. **`#[label("text")]`** — one or more fields of a span type; the label's
   text is a positioned note.

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
once per error; `#[label(collection, "...")]` takes a `Vec` of any
`Into<SourceSpan>` type when the count is unknown — details in
`references/dynamic-diagnostics-and-handlers.md`).

## The advice — `help`

Advice (rules 4, 5, 7) rides on the `help` field, never in the message.
Static per error type:

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

`#[diagnostic(help("..."))]` supports format arguments over the fields, so it
stays dynamic too: `#[diagnostic(help("try `{path}` without the extension"))]`.

## Raising errors

- `miette!("msg")` — build a `Report` from a message (like `anyhow!`).
- `bail!("msg")` — return early with a `Report`: `return Err(miette!("msg").into())`.
- `ensure!(cond, "msg")` — bail unless the condition holds.
- `Err(my_error)?` — a `Diagnostic` error converts into `Report` via `From`.
- `.into_diagnostic()` — wrap any `std::error::Error` into a `Report`.
- `.wrap_err("context")` — `WrapErr`/`Context`, the rule-3 chain.

## Rendering

The fancy report renders once at the top-level handler (rule 6): `main`
returns `miette::Result<()>` and the runtime prints the returned `Err` to
stderr, decorations and unicode disabled when stderr is not a terminal. `{:?}`
on a `Report` renders the same diagnostic anywhere; `{}` prints only the top
message — prefer `{:?}` where you print a report. Set a custom handler with
`miette::set_hook` — see `references/dynamic-diagnostics-and-handlers.md`.

## Pointers

- Read `writing-error-messages` and run it over every message, label, and
  `help` text you write here.
- `#[error]` and variant naming per `rust-code-style`.
- Runtime-built diagnostics, delayed source, multiple related errors, custom
  handlers, and a `#[source_code]` that is not plain owned text (lazy, byte,
  or multi-file sources): read `references/dynamic-diagnostics-and-handlers.md`.
- API: https://docs.rs/miette/latest/miette/