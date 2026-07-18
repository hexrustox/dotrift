use std::io;

use crate::{Template, ast::Node, error::Result};

impl Template {
    /// Renders a sequence of nodes to the writer. This slice only knows text
    /// nodes, emitted verbatim.
    pub(crate) fn eval_body<W: io::Write>(&self, nodes: &[Node], writer: &mut W) -> Result<()> {
        for node in nodes {
            match node {
                Node::Text(range) => writer.write_all(&self.src.bytes()[range.clone()])?,
            }
        }
        Ok(())
    }
}
