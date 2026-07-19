mod ast;
mod error;
mod eval;
mod function;
mod lex;
mod parser;
mod scanner;
mod util;
mod value;

use std::{collections::HashMap, io};

pub use error::{Error, FuncError, ParseError, RenderError, Result};
pub use function::FunctionRegistry;
pub use value::{Value, ValueType};

use ast::Node;

/// A parsed template: the AST plus the source bytes it references.
pub struct Template {
    pub(crate) nodes: Vec<Node>,
    pub(crate) src: Source,
}

/// The backing bytes of a [`Template`].
pub(crate) enum Source {
    Owned(Vec<u8>),
}

impl Source {
    pub(crate) fn bytes(&self) -> &[u8] {
        match self {
            Source::Owned(bytes) => bytes,
        }
    }
}

impl Template {
    /// Constructs a template from an owned byte buffer.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        let tokens = scanner::scan(&bytes)?;
        let nodes = parser::parse(tokens, &bytes)?;
        Ok(Self {
            nodes,
            src: Source::Owned(bytes),
        })
    }

    /// Renders the template to `writer`, flushing it on success.
    pub fn render<W: io::Write>(
        &self,
        mut writer: W,
        variables: &HashMap<String, Value>,
        _functions: &dyn FunctionRegistry,
    ) -> Result<()> {
        let frame = eval::Frame::Var(variables);
        self.eval_body(&self.nodes, &mut writer, &frame)?;
        writer.flush()?;
        Ok(())
    }
}
