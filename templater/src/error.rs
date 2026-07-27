use std::{borrow::Cow, io, sync::Arc};

use miette::{Diagnostic, MietteError, MietteSpanContents, SourceCode, SourceSpan, SpanContents};

use crate::ValueType;

/// Errors raised while constructing or rendering a [`Template`](crate::Template).
#[derive(Debug, thiserror::Error, Diagnostic)]
pub enum Error {
    #[error("{err}")]
    #[diagnostic(code(templater::parse))]
    Parse {
        #[source]
        err: ParseError,
        #[label]
        span: SourceSpan,
        #[source_code]
        source_code: ByteSource<'static>,
    },
    #[error("{err}")]
    #[diagnostic(code(templater::render))]
    Render {
        #[source]
        err: RenderError,
        #[label]
        span: SourceSpan,
        #[source_code]
        source_code: ByteSource<'static>,
    },
    #[error("{err}")]
    #[diagnostic(code(templater::func))]
    Func {
        #[source]
        err: FuncError,
        #[label(primary)]
        span: SourceSpan,
        #[source_code]
        source_code: ByteSource<'static>,
    },
    #[error("{0}")]
    #[diagnostic(code(templater::io))]
    Io(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Constructs a parse error annotated with a source span.
    ///
    /// The source code is left empty; it is filled in at the construction
    /// boundary with the full source bytes.
    pub(crate) fn parse(err: ParseError, span: impl Into<SourceSpan>) -> Self {
        Self::Parse {
            err,
            span: span.into(),
            source_code: ByteSource::Owned(Arc::from(&[][..])),
        }
    }

    /// Constructs a render error annotated with a source span.
    ///
    /// The source code is left empty; it is filled in at the public render
    /// boundary with a span window borrowed from the template's source.
    pub(crate) fn render(err: RenderError, span: impl Into<SourceSpan>) -> Self {
        Self::Render {
            err,
            span: span.into(),
            source_code: ByteSource::Owned(Arc::from(&[][..])),
        }
    }

    /// Constructs a function error annotated with the call-expression span.
    ///
    /// The source code is left empty; it is filled in at the public render
    /// boundary with a span window borrowed from the template's source.
    pub(crate) fn func(err: FuncError, span: impl Into<SourceSpan>) -> Self {
        Self::Func {
            err,
            span: span.into(),
            source_code: ByteSource::Owned(Arc::from(&[][..])),
        }
    }

    /// Replaces the source code attached to a parse/render/function error.
    pub(crate) fn with_source_code(self, source_code: ByteSource<'static>) -> Self {
        match self {
            Self::Parse { err, span, .. } => Self::Parse {
                err,
                span,
                source_code,
            },
            Self::Render { err, span, .. } => Self::Render {
                err,
                span,
                source_code,
            },
            Self::Func { err, span, .. } => Self::Func {
                err,
                span,
                source_code,
            },
            other => other,
        }
    }
}

/// Errors raised during tokenization or parsing, before any content executes.
#[derive(Debug, thiserror::Error, Diagnostic, PartialEq)]
pub enum ParseError {
    #[error("empty interpolation")]
    #[diagnostic(code(templater::parse::empty_interpolation))]
    EmptyInterpolation,
    #[error("integer literal out of i64 range")]
    #[diagnostic(code(templater::parse::integer_out_of_range))]
    IntegerOutOfRange,
    #[error("unclosed string literal")]
    #[diagnostic(code(templater::parse::unclosed_string))]
    UnclosedString,
    #[error("unclosed delimiter")]
    #[diagnostic(code(templater::parse::unclosed_delimiter))]
    UnclosedDelimiter,
    #[error("unclosed function")]
    #[diagnostic(code(templater::parse::unclosed_function))]
    UnclosedFunction,
    #[error("unclosed list")]
    #[diagnostic(code(templater::parse::unclosed_list))]
    UnclosedList,
    #[error("unexpected token")]
    #[diagnostic(code(templater::parse::unexpected_token))]
    UnexpectedToken,
    #[error("unexpected tokens after expression")]
    #[diagnostic(code(templater::parse::unexpected_tokens_after_expr))]
    UnexpectedTokensAfterExpr,
    #[error("trailing comma")]
    #[diagnostic(code(templater::parse::trailing_comma))]
    TrailingComma,
    #[error("empty identifier after `.`")]
    #[diagnostic(code(templater::parse::empty_field))]
    EmptyField,
    #[error("stray delimiter")]
    #[diagnostic(code(templater::parse::stray_delimiter))]
    StrayDelimiter,
    #[error("reserved keyword `{keyword}`")]
    #[diagnostic(code(templater::parse::reserved_keyword))]
    ReservedKeyword { keyword: String },
    #[error("invalid modifier")]
    #[diagnostic(code(templater::parse::invalid_modifier))]
    InvalidModifier,
    #[error("empty statement")]
    #[diagnostic(code(templater::parse::empty_statement))]
    EmptyStatement,
    #[error("unrecognized statement")]
    #[diagnostic(code(templater::parse::unrecognized_statement))]
    UnrecognizedStatement,
    #[error("missing condition")]
    #[diagnostic(code(templater::parse::missing_condition))]
    MissingCondition,
    #[error("empty `for` statement")]
    #[diagnostic(code(templater::parse::empty_for))]
    EmptyFor,
    #[error("not a valid variable name")]
    #[diagnostic(code(templater::parse::invalid_variable))]
    InvalidVariable,
    #[error("missing `in` keyword")]
    #[diagnostic(code(templater::parse::missing_in))]
    MissingIn,
    #[error("missing iterable")]
    #[diagnostic(code(templater::parse::missing_iterable))]
    MissingIterable,
    #[error("unclosed block")]
    #[diagnostic(code(templater::parse::unclosed_block))]
    UnclosedBlock,
    #[error("orphan `end`")]
    #[diagnostic(code(templater::parse::orphan_end))]
    OrphanEnd,
    #[error("`elif` outside of an `if` block")]
    #[diagnostic(code(templater::parse::elif_outside_if))]
    ElifOutsideIf,
    #[error("`else` outside of an `if` block")]
    #[diagnostic(code(templater::parse::else_outside_if))]
    ElseOutsideIf,
}

/// Errors raised only on actually-executed content.
#[derive(Debug, thiserror::Error, Diagnostic, PartialEq)]
pub enum RenderError {
    #[error("undefined variable")]
    #[diagnostic(code(templater::render::undefined_variable))]
    UndefinedVariable,
    #[error("map key `{key}` not found")]
    #[diagnostic(code(templater::render::map_key_not_found))]
    MapKeyNotFound { key: String },
    #[error("list index {idx} out of bounds (length {len})")]
    #[diagnostic(code(templater::render::list_index_out_of_bounds))]
    ListIndexOutOfBounds { idx: i64, len: usize },
    #[error("negative list index {idx}")]
    #[diagnostic(code(templater::render::negative_list_index))]
    NegativeListIndex { idx: i64 },
    #[error("expects {expected}, got {got}")]
    #[diagnostic(code(templater::render::type_mismatch))]
    TypeMismatch { expected: ValueType, got: ValueType },
}

/// Errors returned by the host's [`FunctionRegistry`](crate::FunctionRegistry).
#[derive(Debug, thiserror::Error, Diagnostic, PartialEq)]
pub enum FuncError {
    #[error("undefined function `{name}`")]
    #[diagnostic(code(templater::func::undefined))]
    Undefined { name: String },
    #[error("expects {expected} arguments, got {got}")]
    #[diagnostic(code(templater::func::arg_count))]
    ArgCount { expected: usize, got: usize },
    #[error("expects type {expected}, got {got}")]
    #[diagnostic(code(templater::func::type_mismatch))]
    TypeMismatch {
        expected: ValueType,
        got: ValueType,
        arg_index: usize,
    },
}

/// Source bytes exposed to miette so error spans render byte-accurately.
/// Only the requested span window is ever decoded; the whole source is never
/// lossily converted.
#[derive(Debug)]
pub enum ByteSource<'a> {
    Owned(Arc<[u8]>),
    Borrowed(&'a [u8]),
}

impl ByteSource<'_> {
    /// Returns an owned source-code snippet containing the requested span plus
    /// one line of context on each side.
    pub(crate) fn snippet(src: &[u8], span: SourceSpan) -> ByteSource<'static> {
        let window = span_window_bounds(src, &span, 1, 1);
        let data = match String::from_utf8_lossy(&src[window.start..window.end]) {
            Cow::Borrowed(_) => src[window.start..window.end].to_vec(),
            Cow::Owned(decoded) => decoded.into_bytes(),
        };
        ByteSource::Owned(Arc::from(data))
    }

    fn as_slice(&self) -> &[u8] {
        match self {
            ByteSource::Owned(bytes) => bytes,
            ByteSource::Borrowed(bytes) => bytes,
        }
    }
}

/// Positional and byte-span metadata for a source window.
struct SpanWindow {
    start: usize,
    end: usize,
    line: usize,
    column: usize,
    line_count: usize,
}

/// Computes the byte window and positional metadata for a span.
fn span_window_bounds(
    src: &[u8],
    span: &SourceSpan,
    context_lines_before: usize,
    context_lines_after: usize,
) -> SpanWindow {
    let start = span.offset();
    let end = start + span.len();
    assert!(
        end <= src.len(),
        "span out of bounds: {start}..{end} > {}",
        src.len()
    );

    // Start of the line containing `pos` (`\n` is the only line
    // terminator; `\r` is ordinary text).
    let line_start = |pos: usize| {
        src[..pos]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |i| i + 1)
    };
    // End of the line containing `pos`: through its terminating `\n`,
    // or the end of the source.
    let line_end = |pos: usize| match src[pos..].iter().position(|&b| b == b'\n') {
        Some(i) => pos + i + 1,
        None => src.len(),
    };
    let span_line_start = line_start(start);

    // With zero context the window is exactly the span; otherwise it
    // grows to whole lines, `context_lines_before` above and
    // `context_lines_after` below (miette's own convention).
    let mut win_start = if context_lines_before == 0 {
        start
    } else {
        span_line_start
    };
    if context_lines_before > 0 {
        for _ in 0..context_lines_before {
            if win_start == 0 {
                break;
            }
            win_start = line_start(win_start - 1);
        }
    }

    let win_end = if context_lines_after == 0 {
        end
    } else {
        // Through the newline terminating the span's own line, then
        // `context_lines_after` further lines.
        let mut win_end = line_end(end);
        for _ in 0..context_lines_after {
            if win_end >= src.len() {
                break;
            }
            win_end = line_end(win_end);
        }
        win_end
    };

    let line = src[..win_start].iter().filter(|&&b| b == b'\n').count();
    let column = if context_lines_before == 0 {
        start - span_line_start
    } else {
        0
    };
    let line_count = src[win_start..win_end]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1;

    SpanWindow {
        start: win_start,
        end: win_end,
        line,
        column,
        line_count,
    }
}

impl SourceCode for ByteSource<'_> {
    fn read_span<'a>(
        &'a self,
        span: &SourceSpan,
        context_lines_before: usize,
        context_lines_after: usize,
    ) -> std::result::Result<Box<dyn SpanContents<'a> + 'a>, MietteError> {
        let src = self.as_slice();
        let start = span.offset();
        let end = start + span.len();
        if end > src.len() {
            return Err(MietteError::OutOfBounds);
        }

        let window = span_window_bounds(src, span, context_lines_before, context_lines_after);

        let data: &'a [u8] = match String::from_utf8_lossy(&src[window.start..window.end]) {
            Cow::Borrowed(valid) => valid.as_bytes(),
            // The window holds invalid UTF-8. `SpanContents<'a>` borrows from
            // `self`, so the owned decode can only be returned by leaking —
            // the cold path (an error window over non-UTF-8 bytes), bounded
            // by the window size. The span stays in source coordinates; when
            // decoding changed the window's length, underline alignment
            // inside the window degrades as it does for miette's own
            // raw-bytes impl, whose renderer applies the same lossy decode.
            Cow::Owned(decoded) => Box::leak(decoded.into_bytes().into_boxed_slice()),
        };
        let win_span = SourceSpan::from((window.start, window.end - window.start));
        Ok(Box::new(MietteSpanContents::new(
            data,
            win_span,
            window.line,
            window.column,
            window.line_count,
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use miette::SourceCode;
    use test_case::test_case;

    use super::ByteSource;

    /// Expected fields of a `SpanContents` returned by `ByteSource::read_span`.
    /// Each `#[test_case]` returns a `Window` built from the call; the test
    /// body asserts field-by-field for readable failure messages.
    #[derive(Debug, PartialEq)]
    struct Window {
        data: Vec<u8>,
        offset: usize,
        len: usize,
        line: usize,
        column: usize,
        line_count: usize,
    }

    fn source(bytes: &[u8]) -> ByteSource<'static> {
        ByteSource::Owned(Arc::from(bytes))
    }

    fn borrowed_source(bytes: &[u8]) -> ByteSource<'_> {
        ByteSource::Borrowed(bytes)
    }

    fn read_window(bytes: &[u8], span: (usize, usize), before: usize, after: usize) -> Window {
        let src = source(bytes);
        let contents = src.read_span(&span.into(), before, after).unwrap();
        Window {
            data: contents.data().to_vec(),
            offset: contents.span().offset(),
            len: contents.span().len(),
            line: contents.line(),
            column: contents.column(),
            line_count: contents.line_count(),
        }
    }

    #[test_case(
        b"hello world", (6, 5), 0, 0,
        Window { data: b"world".to_vec(), offset: 6, len: 5, line: 0, column: 6, line_count: 1 } ;
        "exact span with zero context"
    )]
    #[test_case(
        b"one\ntwo\nthree\nfour", (8, 5), 1, 1,
        Window { data: b"two\nthree\nfour".to_vec(), offset: 4, len: 14, line: 1, column: 0, line_count: 3 } ;
        "context window above and below"
    )]
    #[test_case(
        b"aa\nbb", (3, 2), 5, 0,
        Window { data: b"aa\nbb".to_vec(), offset: 0, len: 5, line: 0, column: 0, line_count: 2 } ;
        "clamps window start at start of source"
    )]
    #[test_case(
        b"aa\nbb", (0, 2), 0, 5,
        Window { data: b"aa\nbb".to_vec(), offset: 0, len: 5, line: 0, column: 0, line_count: 2 } ;
        "clamps window end at end of source"
    )]
    #[test_case(
        b"a\xffb", (1, 1), 0, 0,
        Window { data: "\u{fffd}".as_bytes().to_vec(), offset: 1, len: 1, line: 0, column: 1, line_count: 1 } ;
        "decodes invalid utf8 lossy"
    )]
    #[test_case(
        b"", (0, 0), 0, 0,
        Window { data: b"".to_vec(), offset: 0, len: 0, line: 0, column: 0, line_count: 1 } ;
        "empty source empty span"
    )]
    fn read_span_returns_expected_window(
        bytes: &[u8],
        span: (usize, usize),
        before: usize,
        after: usize,
        expected: Window,
    ) {
        let actual = read_window(bytes, span, before, after);
        assert_eq!(actual.data, expected.data);
        assert_eq!(actual.offset, expected.offset);
        assert_eq!(actual.len, expected.len);
        assert_eq!(actual.line, expected.line);
        assert_eq!(actual.column, expected.column);
        assert_eq!(actual.line_count, expected.line_count);
    }

    #[test]
    fn read_span_out_of_bounds_is_an_error() {
        let src = source(b"short");
        assert!(src.read_span(&(10, 5).into(), 0, 0).is_err());
    }

    #[test]
    fn borrowed_read_span_matches_owned() {
        let bytes = b"one\ntwo\nthree";
        let owned = read_window(bytes, (8, 3), 1, 1);
        let borrowed = {
            let src = borrowed_source(bytes);
            let contents = src.read_span(&(8, 3).into(), 1, 1).unwrap();
            Window {
                data: contents.data().to_vec(),
                offset: contents.span().offset(),
                len: contents.span().len(),
                line: contents.line(),
                column: contents.column(),
                line_count: contents.line_count(),
            }
        };
        assert_eq!(owned, borrowed);
    }
}
