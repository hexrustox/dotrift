use std::{borrow::Cow, io, sync::Arc};

use miette::{MietteError, MietteSpanContents, SourceCode, SourceSpan, SpanContents};

/// Errors raised while constructing or rendering a [`Template`](crate::Template).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error(transparent)]
    Func(#[from] FuncError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Errors raised during tokenization or parsing, before any content executes.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {}

/// Errors raised only on actually-executed content.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {}

/// Errors returned by the host's [`FunctionRegistry`](crate::FunctionRegistry).
#[derive(Debug, thiserror::Error)]
pub enum FuncError {
    #[error("undefined function `{name}`")]
    Undefined { name: String },
    #[error("function `{name}` expects {expected} arguments, got {got}")]
    ArgCount {
        name: String,
        expected: String,
        got: usize,
    },
    #[error("function `{name}` argument {arg} has type {got}, expected {expected}")]
    TypeMismatch {
        name: String,
        arg: usize,
        expected: String,
        got: String,
    },
}

/// Source bytes exposed to miette so error spans render byte-accurately.
/// Only the requested span window is ever decoded; the whole source is never
/// lossily converted.
#[allow(dead_code)] // Constructed by the parse-error path from issue 02 onward.
pub(crate) enum ByteSource {
    Owned(Arc<[u8]>),
}

impl SourceCode for ByteSource {
    fn read_span<'a>(
        &'a self,
        span: &SourceSpan,
        context_lines_before: usize,
        context_lines_after: usize,
    ) -> std::result::Result<Box<dyn SpanContents<'a> + 'a>, MietteError> {
        let ByteSource::Owned(bytes) = self;
        let src: &[u8] = bytes;
        let start = span.offset();
        let end = start + span.len();
        if end > src.len() {
            return Err(MietteError::OutOfBounds);
        }

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

        let data: &'a [u8] = match String::from_utf8_lossy(&src[win_start..win_end]) {
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
        let win_span = SourceSpan::from((win_start, win_end - win_start));
        Ok(Box::new(MietteSpanContents::new(
            data, win_span, line, column, line_count,
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use miette::SourceCode;

    use super::ByteSource;

    fn source(bytes: &[u8]) -> ByteSource {
        ByteSource::Owned(Arc::from(bytes))
    }

    #[test]
    fn read_span_with_zero_context_covers_exactly_the_span() {
        let src = source(b"hello world");
        let contents = src.read_span(&(6, 5).into(), 0, 0).unwrap();
        assert_eq!(contents.data(), b"world");
        assert_eq!(contents.span().offset(), 6);
        assert_eq!(contents.span().len(), 5);
        assert_eq!(contents.line(), 0);
        assert_eq!(contents.column(), 6);
    }

    #[test]
    fn read_span_includes_context_lines_above_and_below() {
        let src = source(b"one\ntwo\nthree\nfour");
        let contents = src.read_span(&(8, 5).into(), 1, 1).unwrap();
        assert_eq!(contents.data(), b"two\nthree\nfour");
        assert_eq!(contents.span().offset(), 4);
        assert_eq!(contents.span().len(), 14);
        assert_eq!(contents.line(), 1);
        assert_eq!(contents.column(), 0);
        assert_eq!(contents.line_count(), 3);
    }

    #[test]
    fn read_span_clamps_window_start_at_start_of_source() {
        let src = source(b"aa\nbb");
        let contents = src.read_span(&(3, 2).into(), 5, 0).unwrap();
        assert_eq!(contents.data(), b"aa\nbb");
        assert_eq!(contents.span().offset(), 0);
        assert_eq!(contents.line(), 0);
    }

    #[test]
    fn read_span_clamps_window_end_at_end_of_source() {
        let src = source(b"aa\nbb");
        let contents = src.read_span(&(0, 2).into(), 0, 5).unwrap();
        assert_eq!(contents.data(), b"aa\nbb");
        assert_eq!(contents.span().offset(), 0);
        assert_eq!(contents.span().len(), 5);
        assert_eq!(contents.line_count(), 2);
    }

    #[test]
    fn read_span_out_of_bounds_is_an_error() {
        let src = source(b"short");
        assert!(src.read_span(&(10, 5).into(), 0, 0).is_err());
    }

    #[test]
    fn read_span_decodes_invalid_utf8_lossily() {
        let src = source(b"a\xffb");
        let contents = src.read_span(&(1, 1).into(), 0, 0).unwrap();
        assert_eq!(contents.data(), "\u{fffd}".as_bytes());
    }

    #[test]
    fn read_span_empty_source_empty_span() {
        let src = source(b"");
        let contents = src.read_span(&(0, 0).into(), 0, 0).unwrap();
        assert_eq!(contents.data(), b"");
    }
}
