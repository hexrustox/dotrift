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

use crate::{ast::Node, function::FunctionRegistry};
pub use crate::{
    error::{FuncError, RenderError},
    eval::{EvalContext, eval_nodes},
    value::Value,
};

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

pub struct Template {
    source: Source,
    nodes: Vec<Node>,
}

impl Template {
    pub fn from_mmap(mmap: Mmap) -> miette::Result<Self> {
        let tokens = scanner::scan(&mmap)?;
        let nodes = parser::parse(&tokens, &mmap)?;
        Ok(Self {
            source: Source::Mapped(mmap),
            nodes,
        })
    }

    pub fn from_bytes(source: Vec<u8>) -> miette::Result<Self> {
        let tokens = scanner::scan(&source)?;
        let nodes = parser::parse(&tokens, &source)?;
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
    ) -> Result<(), RenderError> {
        let mut ctx = EvalContext::new(variables, functions);
        let mut writer = io::BufWriter::new(writer);
        eval_nodes(&self.nodes, self.source.as_bytes(), &mut writer, &mut ctx)
    }
}
