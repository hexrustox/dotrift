mod ast;
mod error;
pub mod parser;
pub mod scanner;

use std::fs::File;
use std::path::Path;

use anyhow::Context;
use memmap2::Mmap;

use crate::ast::Node;

pub struct Template {
    mmap: Mmap,
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
        Ok(Self { mmap, nodes })
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn source(&self) -> &[u8] {
        &self.mmap
    }
}
