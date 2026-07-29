use std::io;

use miette::{Diagnostic, SourceSpan};

use crate::ValueType;

/// Errors raised while constructing or rendering a [`Template`](crate::Template).
#[derive(Debug, thiserror::Error, Diagnostic)]
pub enum Error {
    #[error("{err}")]
    Parse {
        err: ParseError,
        #[label]
        span: SourceSpan,
    },
    #[error("{err}")]
    Render {
        err: RenderError,
        #[label]
        span: SourceSpan,
    },
    #[error("{err}")]
    Func {
        err: FuncError,
        #[label]
        span: SourceSpan,
    },
    #[error("{0}")]
    Io(#[from] io::Error),
}

impl Error {
    /// Constructs a parse error annotated with a source span.
    ///
    /// The source code is left empty; it is filled in at the construction
    /// boundary with the full source bytes.
    pub(crate) fn parse(err: ParseError, span: impl Into<SourceSpan>) -> Self {
        Self::Parse {
            err,
            span: span.into(),
        }
    }

    /// Constructs a render error annotated with a source span.
    ///
    /// The source code is left empty; it is filled in at the public render
    /// boundary with a span window borrowed from the template's source.
    pub(crate) fn render(err: RenderError, span: impl Into<SourceSpan>) -> Self {
        Self::Render {
            err,
            span: span.into(),
        }
    }

    /// Constructs a function error annotated with the call-expression span.
    ///
    /// The source code is left empty; it is filled in at the public render
    /// boundary with a span window borrowed from the template's source.
    pub(crate) fn func(err: FuncError, span: impl Into<SourceSpan>) -> Self {
        Self::Func {
            err,
            span: span.into(),
        }
    }
}

/// Errors raised during tokenization or parsing, before any content executes.
#[derive(Debug, thiserror::Error, Diagnostic, PartialEq)]
pub enum ParseError {
    #[error("empty interpolation")]
    EmptyInterpolation,
    #[error("integer literal out of i64 range")]
    IntegerOutOfRange,
    #[error("unclosed string literal")]
    UnclosedString,
    #[error("unclosed delimiter")]
    UnclosedDelimiter,
    #[error("unclosed function")]
    UnclosedFunction,
    #[error("unclosed list")]
    UnclosedList,
    #[error("unexpected token")]
    UnexpectedToken,
    #[error("unexpected tokens after expression")]
    UnexpectedTokensAfterExpr,
    #[error("trailing comma")]
    TrailingComma,
    #[error("empty identifier after `.`")]
    EmptyField,
    #[error("stray delimiter")]
    StrayDelimiter,
    #[error("reserved keyword `{keyword}`")]
    ReservedKeyword { keyword: String },
    #[error("invalid modifier")]
    InvalidModifier,
    #[error("empty statement")]
    EmptyStatement,
    #[error("unrecognized statement")]
    UnrecognizedStatement,
    #[error("missing condition")]
    MissingCondition,
    #[error("empty `for` statement")]
    EmptyFor,
    #[error("not a valid variable name")]
    InvalidVariable,
    #[error("missing `in` keyword")]
    MissingIn,
    #[error("missing iterable")]
    MissingIterable,
    #[error("unclosed block")]
    UnclosedBlock,
    #[error("orphan `end`")]
    OrphanEnd,
    #[error("`elif` outside of an `if` block")]
    ElifOutsideIf,
    #[error("`else` outside of an `if` block")]
    ElseOutsideIf,
}

/// Errors raised only on actually-executed content.
#[derive(Debug, Clone, thiserror::Error, Diagnostic, PartialEq)]
pub enum RenderError {
    #[error("undefined variable")]
    UndefinedVariable,
    #[error("map key `{key}` not found")]
    MapKeyNotFound { key: String },
    #[error("list index {idx} out of bounds (length {len})")]
    ListIndexOutOfBounds { idx: i64, len: usize },
    #[error("negative list index {idx}")]
    NegativeListIndex { idx: i64 },
    #[error("expected type {expected}, got {got}")]
    TypeMismatch { expected: ValueType, got: ValueType },
}

/// Errors returned by the host's [`FunctionRegistry`](crate::FunctionRegistry).
#[derive(Debug, thiserror::Error, Diagnostic, PartialEq)]
pub enum FuncError {
    #[error("undefined function `{name}`")]
    Undefined { name: String },
    #[error("expected {expected} arguments, got {got}")]
    ArgCount { expected: usize, got: usize },
    #[error("expected type {expected}, got {got}")]
    TypeMismatch {
        expected: ValueType,
        got: ValueType,
        arg_index: usize,
    },
}
