mod ast;
pub mod error;
mod eval;
pub mod function;
mod parser;
mod scanner;
mod trim;
pub mod util;
pub mod value;

use std::{collections::HashMap, fs::File, io, path::Path, sync::Arc};

use memmap2::Mmap;
use miette::{Report, SourceCode};

use crate::error::{Error, RegistryError};
use crate::{function::FunctionRegistry, value::Value};

/// A parsed template: the AST plus the source bytes it references.
#[derive(Debug)]
pub struct Template {
    src: Source,
}

/// The backing bytes of a [`Template`].
#[derive(Debug, Clone)]
enum Source {
    Bytes(Vec<u8>),
    Mapped(std::sync::Arc<Mmap>),
}

impl Source {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Source::Bytes(bytes) => bytes,
            Source::Mapped(mmap) => mmap,
        }
    }
}

impl SourceCode for Source {
    fn read_span<'a>(
        &'a self,
        span: &miette::SourceSpan,
        context_lines_before: usize,
        context_lines_after: usize,
    ) -> Result<Box<dyn miette::SpanContents<'a> + 'a>, miette::MietteError> {
        let src = self.as_bytes();
        let span_offset = span.offset();
        let span_len = span.len();
        let span_end = span_offset + span_len;
        if span_end > src.len() {
            return Err(miette::MietteError::OutOfBounds);
        }

        // Find the start of the line containing span_offset.
        let line_start = |pos: usize| {
            src[..pos]
                .iter()
                .rposition(|&b| b == b'\n')
                .map_or(0, |i| i + 1)
        };

        // Find the end of the line containing pos (through its \n or EOF).
        let line_end = |pos: usize| match src[pos..].iter().position(|&b| b == b'\n') {
            Some(i) => pos + i + 1,
            None => src.len(),
        };

        // Compute window start: the start of the line `context_lines_before` lines above
        // the span's line, clamped at SOF.
        let mut window_start = line_start(span_offset);
        for _ in 0..context_lines_before {
            if window_start == 0 {
                break;
            }
            window_start = line_start(window_start - 1);
        }

        // Compute window end: the end of the line `context_lines_after` lines below
        // the span's line, clamped at EOF.
        let mut window_end = line_end(span_offset);
        for _ in 0..context_lines_after {
            if window_end >= src.len() {
                break;
            }
            window_end = line_end(window_end);
        }

        // The line number to report is the number of lines before window_start.
        let line = src[..window_start].iter().filter(|&&b| b == b'\n').count();

        // Column is the offset of span_offset from the start of its own line.
        let span_line_start = line_start(span_offset);
        let column = span_offset - span_line_start;

        // Total line count in the window.
        let line_count = src[window_start..window_end]
            .iter()
            .filter(|&&b| b == b'\n')
            .count()
            + 1;

        // Data is the borrowed subslice.
        let data = &src[window_start..window_end];

        // The returned span must be relative to data.
        let relative_span = miette::SourceSpan::from((window_start, 0));

        Ok(Box::new(miette::MietteSpanContents::new(
            data,
            relative_span,
            line,
            column,
            line_count,
        )))
    }
}

impl Template {
    /// Constructs a template from an owned byte buffer.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            src: Source::Bytes(bytes.into()),
        }
    }

    /// Constructs a template by memory-mapping the file at `path` and parsing
    /// its contents.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, std::io::Error> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file) }?;
        Ok(Self {
            src: Source::Mapped(Arc::new(mmap)),
        })
    }

    /// Renders the template to `writer`, flushing it on success.
    ///
    /// On failure, returns a [`miette::Report`] whose source code contains the
    /// span window surrounding the error.
    pub fn render<W: io::Write>(
        &self,
        mut writer: W,
        variables: &HashMap<String, Value>,
        functions: &dyn FunctionRegistry,
    ) -> Result<(), Error> {
        let bytes = self.src.as_bytes();
        let mut tokens = scanner::scan(bytes)?;
        trim::trim_tokens(&mut tokens, bytes);
        let nodes = parser::parse(tokens, bytes)?;
        let mut scope = eval::Scope::new(variables);
        self.eval_body(&nodes, &mut writer, &mut scope, functions)?;
        writer.flush()?;
        Ok(())
    }

    pub fn report(self, result: Result<(), Error>) -> Result<(), Report> {
        result.map_err(|e| Report::new(e).with_source_code(self.src))
    }
}
