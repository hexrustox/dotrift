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

use crate::{ast::Node, error::ParseError, function::FunctionRegistry};
pub use crate::{
    error::{FuncError, RenderError},
    eval::{EvalContext, eval_nodes},
    value::Value,
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
    pub fn from_mmap(mmap: Mmap) -> miette::Result<Self> {
        let nodes = annotate(&mmap, parse(&mmap))?;
        Ok(Self {
            source: Source::Mapped(mmap),
            nodes,
        })
    }

    pub fn from_bytes(source: Vec<u8>) -> miette::Result<Self> {
        let nodes = annotate(&source, parse(&source))?;
        Ok(Self {
            source: Source::Owned(source),
            nodes,
        })
    }

    pub fn render<W: io::Write>(
        &self,
        writer: W,
        variables: HashMap<String, Value>,
        functions: &dyn FunctionRegistry,
    ) -> miette::Result<()> {
        let mut ctx = EvalContext::new(variables, functions);
        let mut writer = io::BufWriter::new(writer);
        let source = self.source.as_bytes();
        eval_nodes(&self.nodes, source, &mut writer, &mut ctx).map_err(|e| match e {
            RenderError::Eval(mut e) => {
                let (at, len) = e.get_at();
                let (src, adj) = source_context(at, len, source);
                e.set_at(adj, len);
                miette::Report::new(e).with_source_code(src)
            }
            RenderError::Io(e) => miette::miette!("{e}"),
        })
    }
}

fn parse(source: &[u8]) -> miette::Result<Vec<Node>> {
    let tokens = scanner::scan(source)?;
    Ok(parser::parse(&tokens, source)?)
}

fn annotate<T>(source: &[u8], result: miette::Result<T>) -> miette::Result<T> {
    if let Err(mut e) = result {
        let pe = e.downcast_mut::<ParseError>().unwrap();
        let (at, len) = pe.get_at();
        let (src, adj) = source_context(at, len, source);
        pe.set_at(adj, len);
        Err(e.with_source_code(src))
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
