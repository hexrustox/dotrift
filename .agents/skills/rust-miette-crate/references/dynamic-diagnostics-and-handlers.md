# Dynamic diagnostics and handler customization

The derive forms in the skill cover error types known at compile time. This
reference covers diagnostics built at runtime, delayed source, multiple
related errors, and customizing how reports render.

## Dynamic diagnostics: `miette!`, `diagnostic!`, `MietteDiagnostic`

Use these when the errors are not known up front — ad-hoc messages, errors
built from parsed or external data, or errors that must (de)serialize.

`miette!` builds a `Report` from a message plus any combination of metadata
keys, message first or last:

```rust
use miette::{miette, LabeledSpan};

let report = miette!(
    labels = vec![
        LabeledSpan::at(12..13, "this should be 6"),
    ],
    help = "'*' has greater precedence than '+'",
    "wrong answer"
)
.with_source_code("2 + 2 * 2 = 8".to_string());
```

`diagnostic!` does the same but yields a `MietteDiagnostic` (a plain struct
you can inspect, modify, and `.into()` a `Report`); `MietteDiagnostic` fields
also `serde`-round-trip under the `serde` feature. Both accept `severity`,
`code`, `help`, `labels`, and `url`, plus format arguments for the message:

```rust
use miette::{diagnostic, LabeledSpan, Severity};

let diag = diagnostic!(
    severity = Severity::Error,
    code = "expected::rparen",
    help = "always close your parens",
    labels = vec![LabeledSpan::at_offset(6, "here")],
    url = "https://example.com",
    "expected closing ')'",
);
```

### `LabeledSpan` and source

`LabeledSpan` (`offset` + `len` + optional text) is the runtime counterpart of
the derived `#[label]`:

- `LabeledSpan::at(range, label)` — from a `Range`/`RangeInclusive` of byte
  offsets and a label string.
- `LabeledSpan::at_offset(offset, label)` — zero-length marker at a byte
  offset.
- `LabeledSpan::new(Option<String>, offset, len)` — full control, or `None`
  label for an unlabeled marker.

Labels in the derive are **byte** offsets into the `#[source_code]`; the
runtime macros work the same — build `LabeledSpan`s from byte offsets into the
string you pass to `.with_source_code(...)`.

## Collection labels

When a label's count is unknown until runtime, use the `collection` parameter
in the derive together with any `Vec` of `Into<SourceSpan>`:

```rust
#[error("oops")]
pub struct MyError {
    #[source_code]
    src: String,
    #[label("main issue")]
    primary: SourceSpan,
    #[label(collection, "related to this")]
    secondary: Vec<usize>,
}
```

A `Vec<LabeledSpan>` gives differing text per span; a `None` text falls back to
the attribute's label: `LabeledSpan::new(None, 30, 11)`, `Some("and this")`
for a custom one.

## Delayed source code

When the source is only available at the reporting boundary — say a file was
read after parsing failed — leave the label field with no `#[source_code]` in
the type and attach the source at `main`, where it is known:

```rust
fn main() -> miette::Result<()> {
    let src = std::fs::read_to_string("input.txt")?;
    parse(&src).map_err(|err| err.with_source_code(src))?;
    Ok(())
}
```

`.with_source_code(...)` also works on a hand-built `Report` from `miette!`.

## Custom sources — implementing `SourceCode`

`#[source_code]` fields and `.with_source_code(...)` accept any type
implementing `SourceCode` (a `Send + Sync` trait). Blanket impls cover the
common cases: `String`, `&str` / `str` (a borrowed `src: &'a str` field
works), `Vec<u8>` / `[u8]`, `Cow<T>`, `Arc<T>`, and `NamedSource<S>`.

Implement it yourself only when the source is not a materialized string: a
buffer too large to hold whole (the trait reads *only* the requested span plus
its context lines, so lazy and streamed sources work), an index into a
centralized SourceMap across many files, or when line/column/name must be
computed rather than stored.

The one required method asks miette for the bytes it needs and you hand back
what those bytes mean:

```text
fn read_span<'a>(&'a self, span: &SourceSpan,
                 context_lines_before: usize, context_lines_after: usize)
    -> Result<Box<dyn SpanContents<'a> + 'a>, MietteError>;
```

Return a `MietteSpanContents` built with `new(data, span, line, column,
line_count)` (all line/column values 0-indexed) or `new_named(name, ...)` for
a file name, and `.with_language("...")` to drive syntax highlighting. The
renderer reads `data`, `span`, `line`, `column`, `line_count`, `name`, and
`language` off the result for the snippet's line numbers and file header — so
this is exactly where a cross-file offset becomes the right file and line.
Give each file in a SourceMap a distinct offset range and return that file's
name and corrected line from `read_span`; a `#[related]` parent can then share
one such source across several sub-errors. The impl must be `Send + Sync` —
hold an internal buffer in an `Arc` or a `Mutex` if it is shared.

## Multiple related errors

Tag an `IntoIter` field with `#[related]` to render sub-errors together under
one diagnostic — useful for validation that collects every failure instead of
stopping at the first:

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

Each `ConfigError` may carry its own `#[label]`; all of them draw against the
parent's `#[source_code]`.

## Error codes and links

`#[diagnostic(code(app::error_name))]` gives a unique machine-readable code
rendered with the report. With a `url()` it becomes a clickable link:
`url(docsrs)` links to the error type's own docs.rs page, a format string to
an arbitrary URL: `url("https://my_site.dev/errors#{}", self.code().unwrap())`.
Keep codes stable and documented — they are the user-facing handle for the
error.

## Custom handlers and features

The default rendering handler is `MietteHandler`, configurable per-process
with `set_hook`:

```rust
miette::set_hook(Box::new(|_| {
    Box::new(
        miette::MietteHandlerOpts::new()
            .terminal_links(true)
            .unicode(false)
            .context_lines(3)
            .tab_width(4)
            .break_words(true)
            .build(),
    )
}))?;
```

Useful options: `.terminal_links`, `.unicode`, `.context_lines`,
`.force_graphical(..)` / `.force_narrated(..)` (override the auto-detect
between graphical and plain rendering), `.with_syntax_highlighting(..)` /
`.without_syntax_highlighting()` (the `syntect-highlighter` feature turns
highlighting on by default; with it, use the former for a custom highlighter
and the latter to disable). Common adjustments stay one-liners on
`MietteHandlerOpts`; a fully custom printer (e.g. machine-readable output)
implements the `ReportHandler` trait — `JSONReportHandler` ships for JSON and
`NarratableReportHandler` is the screen-reader/plain-text printer selected
automatically off-TTY or under `NO_COLOR`.