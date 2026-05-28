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
}

#[derive(Debug, Diagnostic, Error, PartialEq)]
#[error("{kind}")]
pub struct Error {
    #[source_code]
    src: String,
    kind: ErrorKind,
    #[label]
    at: SourceSpan,
}

impl Error {
    pub fn new(kind: ErrorKind, at: usize, len: usize, source: &[u8]) -> Self {
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
        Error {
            kind,
            at: (at - start, len).into(),
            src: String::from_utf8_lossy(&source[start..end]).into_owned(),
        }
    }

    #[cfg(test)]
    pub fn destruct(self) -> (ErrorKind, usize, usize) {
        (self.kind, self.at.offset(), self.at.len())
    }
}
