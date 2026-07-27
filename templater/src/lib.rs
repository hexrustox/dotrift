mod ast;
mod error;
mod eval;
mod function;
mod parser;
mod scanner;
mod trim;
pub mod util;
mod value;

use std::{collections::HashMap, fs::File, io, path::Path};

pub use error::{ByteSource, Error, FuncError, ParseError, RenderError, Result};
pub use function::FunctionRegistry;
use memmap2::Mmap;
use miette::Report;
pub use value::{Value, ValueType};

use ast::Node;

/// A parsed template: the AST plus the source bytes it references.
#[derive(Debug)]
pub struct Template {
    pub(crate) nodes: Vec<Node>,
    pub(crate) src: Source,
}

/// The backing bytes of a [`Template`].
#[derive(Debug)]
pub(crate) enum Source {
    Owned(Vec<u8>),
    Mapped(Mmap),
}

impl Source {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        match self {
            Source::Owned(bytes) => bytes,
            Source::Mapped(mmap) => mmap,
        }
    }
}

impl Template {
    /// Constructs a template from an owned byte buffer.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        let nodes = parse(&bytes)?;
        Ok(Self {
            nodes,
            src: Source::Owned(bytes),
        })
    }

    /// Constructs a template by memory-mapping the file at `path` and parsing
    /// its contents.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file) }?;
        let nodes = parse(&mmap)?;
        Ok(Self {
            nodes,
            src: Source::Mapped(mmap),
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
    ) -> std::result::Result<(), Report> {
        let mut scope = eval::Scope::new(variables);
        match self.eval_body(&self.nodes, &mut writer, &mut scope, functions) {
            Ok(()) => {}
            Err(err) => {
                let span = match &err {
                    Error::Render { span, .. } | Error::Func { span, .. } => *span,
                    _ => return Err(Report::new(err)),
                };
                let source_code = ByteSource::snippet(self.src.as_bytes(), span);
                return Err(Report::new(err.with_source_code(source_code)));
            }
        }
        writer.flush().map_err(|e| Report::new(Error::Io(e)))?;
        Ok(())
    }
}

fn parse(bytes: &[u8]) -> Result<Vec<Node>> {
    let mut tokens = scanner::scan(bytes)?;
    trim::trim_tokens(&mut tokens, bytes);
    parser::parse(tokens, bytes)
}
