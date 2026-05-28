mod ast;
mod error;
mod eval;
pub mod function;
mod parser;
mod scanner;
pub mod value;

use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::path::Path;

use anyhow::Context;
use memmap2::Mmap;

use crate::ast::Node;
pub use crate::error::EvalError;
pub use crate::eval::EvalContext;
pub use crate::eval::eval_nodes;
pub use crate::value::Value;

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
    pub fn from_path(path: &Path) -> anyhow::Result<Self> {
        let file =
            File::open(path).with_context(|| format!("failed to open `{}`", path.display()))?;
        let mmap = unsafe { Mmap::map(&file) }
            .with_context(|| format!("failed to mmap `{}`", path.display()))?;
        let tokens = scanner::scan(&mmap)?;
        let nodes = parser::parse(&tokens, &mmap)?;
        Ok(Self {
            source: Source::Mapped(mmap),
            nodes,
        })
    }

    pub fn from_bytes(source: Vec<u8>) -> anyhow::Result<Self> {
        let tokens = scanner::scan(&source)?;
        let nodes = parser::parse(&tokens, &source)?;
        Ok(Self {
            source: Source::Owned(source),
            nodes,
        })
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn source(&self) -> &[u8] {
        self.source.as_bytes()
    }

    pub fn render<W: io::Write>(
        &self,
        writer: W,
        variables: HashMap<String, Value>,
        functions: &dyn function::FunctionRegistry,
    ) -> Result<(), EvalError> {
        let mut ctx = EvalContext::new(variables, functions);
        let mut writer = io::BufWriter::new(writer);
        eval_nodes(&self.nodes, self.source(), &mut writer, &mut ctx)
    }
}
