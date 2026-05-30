use std::io;
use std::ops::Range;

use miette::{Diagnostic, SourceSpan};
use thiserror::Error;

#[derive(Debug, Clone, Error, PartialEq)]
pub enum ErrorKind {
    #[error("unclosed delimiter")]
    UnclosedDelimiter,

    #[error("stray closing delimiter")]
    StrayDelimiter,

    #[error("unexpected tokens after expression")]
    UnexpectedTokensAfterExpr,

    #[error("empty statement")]
    EmptyStatement,

    #[error("stray {{% end %}} without matching opening block")]
    StrayEnd,

    #[error("stray {{% elif %}} without matching {{% if %}}")]
    StrayElif,

    #[error("stray {{% else %}} without matching {{% if %}}")]
    StrayElse,

    #[error("unexpected keyword in statement")]
    UnexpectedKeyword,

    #[error("unclosed block")]
    UnclosedBlock,

    #[error("expected {{% for var in expr %}}")]
    ExpectedForSyntax,

    #[error("expected variable name after 'for'")]
    ExpectedForVar,

    #[error("expected 'in' after for variable")]
    ExpectedForIn,

    #[error("unclosed {{% for %}} block")]
    UnclosedFor,

    #[error("expected field name after '.'")]
    ExpectedFieldName,

    #[error("unexpected end of expression")]
    UnexpectedEndOfExpr,

    #[error("expected ',' in list")]
    ExpectedCommaInList,

    #[error("expected ',' between arguments")]
    ExpectedCommaBetweenArgs,

    #[error("unexpected token")]
    UnexpectedToken,

    #[error("unclosed string literal")]
    UnclosedString,

    #[error("unclosed list")]
    UnclosedList,

    #[error("unclosed grouping")]
    UnclosedGroup,

    #[error("undefined variable `{0}`")]
    UndefinedVariable(String),

    #[error("list index out of bounds: index {index} with length {len}")]
    IndexOutOfBounds { index: usize, len: usize },

    #[error("map key not found `{0}`")]
    MapKeyNotFound(String),

    #[error("invalid field access on {ty}: `{field}`")]
    InvalidFieldAccess { ty: &'static str, field: String },

    #[error("{0}")]
    Function(FuncError),
}

#[derive(Debug, Clone, Diagnostic, Error)]
#[error("{kind}")]
pub struct Error {
    kind: ErrorKind,
    #[label]
    at: SourceSpan,
}

impl Error {
    pub fn new(kind: ErrorKind, at: usize, len: usize) -> Self {
        Error {
            kind,
            at: (at, len).into(),
        }
    }

    pub fn from_range(kind: ErrorKind, range: Range<usize>) -> Self {
        Error::new(kind, range.start, range.end - range.start)
    }

    pub fn get_at(&self) -> (usize, usize) {
        (self.at.offset(), self.at.len())
    }

    pub fn set_at(&mut self, offset: usize, len: usize) {
        self.at = (offset, len).into();
    }

    #[cfg(test)]
    pub fn destruct(self) -> (ErrorKind, usize, usize) {
        (self.kind, self.at.offset(), self.at.len())
    }
}

#[derive(Debug, Clone, Error, PartialEq)]
pub enum FuncError {
    #[error("undefined function `{0}`")]
    Undefined(String),

    #[error("type mismatch: expected {expected}, got {got}")]
    TypeMismatch {
        arg: Option<usize>,
        expected: &'static str,
        got: &'static str,
    },

    #[error("wrong argument count for `{name}`: expected {expected}, got {got}")]
    WrongArgCount {
        name: String,
        expected: String,
        got: usize,
    },

    #[error("{0}")]
    Custom(String),
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("{0}")]
    Eval(Error),

    #[error("{0}")]
    Io(#[from] io::Error),
}

impl From<Error> for RenderError {
    fn from(e: Error) -> Self {
        RenderError::Eval(e)
    }
}
