pub mod ast;
pub mod error;
pub mod parser;
pub mod scanner;

use std::fs::File;
use std::path::Path;

use memmap2::Mmap;

use crate::ast::Node;
use crate::error::Error;

pub struct Template {
    mmap: Mmap,
    nodes: Vec<Node>,
}

impl Template {
    pub fn from_path(path: &Path) -> Result<Self, Error> {
        let file = File::open(path)
            .map_err(|e| Error::new(format!("failed to open `{}`: {}", path.display(), e), 0, 0))?;

        let mmap = unsafe { Mmap::map(&file) }
            .map_err(|e| Error::new(format!("failed to mmap `{}`: {}", path.display(), e), 0, 0))?;

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
