use std::io;
use std::ops::Range;

use miette::{Diagnostic, SourceSpan};
use thiserror::Error;

#[derive(Debug, Clone, Error, PartialEq)]
pub enum ParseErrorKind {
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
}

#[derive(Debug, Error, PartialEq)]
pub enum FuncError {
    #[error("undefined function `{0}`")]
    Undefined(String),

    #[error("type mismatch: expected {expected}, got {got}")]
    TypeMismatch {
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

#[derive(Debug, Error, PartialEq)]
pub enum EvalErrorKind {
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

#[derive(Debug, Error)]
#[error("{kind}")]
pub struct EvalError {
    kind: EvalErrorKind,
    range: Range<usize>,
}

impl EvalError {
    pub fn new(kind: EvalErrorKind, range: Range<usize>) -> Self {
        Self { kind, range }
    }

    #[cfg(test)]
    pub fn destruct(self) -> (EvalErrorKind, Range<usize>) {
        (self.kind, self.range)
    }
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("{0}")]
    Eval(EvalError),

    #[error("{0}")]
    Io(#[from] io::Error),
}

impl From<EvalError> for RenderError {
    fn from(e: EvalError) -> Self {
        RenderError::Eval(e)
    }
}

#[derive(Debug, Diagnostic, Error, PartialEq)]
#[error("{kind}")]
pub struct ParseError {
    #[source_code]
    src: String,
    kind: ParseErrorKind,
    #[label]
    at: SourceSpan,
}

impl ParseError {
    pub fn new(kind: ParseErrorKind, at: usize, len: usize, source: &[u8]) -> Self {
        let start = source[..at]
            .iter()
            .rposition(|b| *b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        let end = source[at + len..]
            .iter()
            .position(|b| *b == b'\n')
            .map(|i| i + at + len)
            .unwrap_or(source.len());
        ParseError {
            kind,
            at: (at - start, len).into(),
            src: String::from_utf8_lossy(&source[start..end]).into_owned(),
        }
    }

    #[cfg(test)]
    pub fn destruct(self) -> (ParseErrorKind, usize, usize) {
        (self.kind, self.at.offset(), self.at.len())
    }
}
