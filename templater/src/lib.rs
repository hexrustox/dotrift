mod ast;
mod error;
mod eval;
pub mod function;
mod parser;
mod scanner;
pub mod value;

use std::collections::HashMap;
use std::io;

use memmap2::Mmap;
use miette::{Report, Result};

use crate::{
    ast::Node,
    error::Error,
    eval::{EvalContext, eval_nodes},
    function::FunctionRegistry,
};
pub use crate::{
    error::FuncError,
    value::{Value, ValueType},
};

#[derive(Debug)]
enum Source {
    Mapped(Mmap),
    Owned(Vec<u8>),
}

impl Source {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Source::Mapped(m) => m,
            Source::Owned(v) => v,
        }
    }
}

#[derive(Debug)]
pub struct Template {
    source: Source,
    nodes: Vec<Node>,
}

impl Template {
    pub fn from_mmap(mmap: Mmap) -> Result<Self> {
        let nodes = annotate(&mmap, parse(&mmap))?;
        Ok(Self {
            source: Source::Mapped(mmap),
            nodes,
        })
    }

    pub fn from_bytes(source: Vec<u8>) -> Result<Self> {
        let nodes = annotate(&source, parse(&source))?;
        Ok(Self {
            source: Source::Owned(source),
            nodes,
        })
    }

    pub fn render<W: io::Write>(
        self,
        writer: W,
        variables: &HashMap<String, Value>,
        functions: &dyn FunctionRegistry,
    ) -> Result<()> {
        let mut ctx = EvalContext::new(variables, functions);
        let mut writer = io::BufWriter::new(writer);
        let source = self.source.as_bytes();
        eval_nodes(&self.nodes, source, &mut writer, &mut ctx).map_err(|report| {
            if let Some(error) = report.downcast_ref::<Error>() {
                let (at, len) = error.get_at();
                let (src, adj) = source_context(at, len, source);
                let mut error = error.clone();
                error.set_at(adj, len);
                return Report::new(error).with_source_code(src);
            }
            report
        })?;
        Ok(())
    }
}

fn parse(source: &[u8]) -> Result<Vec<Node>> {
    let tokens = scanner::scan(source)?;
    parser::parse(&tokens, source)
}

fn annotate<T>(source: &[u8], result: Result<T>) -> Result<T> {
    if let Err(mut report) = result {
        let error = report
            .downcast_mut::<Error>()
            .expect("expected templater::error::Error");
        let (at, len) = error.get_at();
        let (src, adj) = source_context(at, len, source);
        error.set_at(adj, len);
        Err(report.with_source_code(src))
    } else {
        result
    }
}

fn source_context(at: usize, len: usize, source: &[u8]) -> (String, usize) {
    let start = source[..at]
        .iter()
        .rposition(|b| *b == b'\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let end = (at + len..source.len())
        .position(|i| source[i] == b'\n')
        .map(|i| at + len + i)
        .unwrap_or(source.len());
    (
        String::from_utf8_lossy(&source[start..end]).into_owned(),
        at.saturating_sub(start),
    )
}
