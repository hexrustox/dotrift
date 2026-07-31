//! Error display conventions:
//!
//! - **Case**: lowercase sentence fragments, no trailing period.
//! - **Quoting**: backtick-quote syntax elements (keywords, delimiters, modifiers,
//!   operators): `` `keyword` ``.
//! - **Field interpolation**: `{field}` for dynamic values (identifiers, types,
//!   indices).
//! - **Voice**: describe what the error *is*, not what the user should *do*
//!   (e.g. "undefined variable" not "variable is not defined").
//! - **Terminology**: prefer spec terms from `CONTEXT.md` (e.g. "interpolation"
//!   not "placeholder", "delimiter" not "bracket").
//! - **Message vs Label**: `#[error("...")]` describes what the error *is*
//!   (descriptive). `#[label("...")]` on `span` tells the user what they
//!   *should do* (prescriptive, actionable). When the action is obvious from
//!   the message, the label text may be omitted and only the underline
//!   remains.

use std::io;

use miette::{Diagnostic, SourceSpan};

use crate::ValueType;

/// Errors raised while constructing or rendering a [`Template`](crate::Template).
#[derive(Debug, thiserror::Error, Diagnostic)]
#[error("{0}")]
pub enum Error {
    #[diagnostic(transparent)]
    Parse(ParseError),
    #[diagnostic(transparent)]
    Render(RenderError),
    Io(#[from] io::Error),
}

/// Errors raised during tokenization or parsing, before any content executes.
#[derive(Debug, thiserror::Error, Diagnostic, PartialEq)]
pub enum ParseError {
    #[error("empty interpolation")]
    EmptyInterpolation {
        #[label("provide an expression")]
        span: SourceSpan,
    },
    #[error("integer literal out of range")]
    IntegerOutOfRange {
        #[label("use a value between {} and {}", i64::MIN, i64::MAX)]
        span: SourceSpan,
    },
    #[error("unclosed string literal")]
    UnclosedString {
        #[label("close the string with `\"`")]
        span: SourceSpan,
    },
    #[error("unclosed delimiter")]
    UnclosedDelimiter {
        delimiter: String,
        #[label("close the tag with `{delimiter}`")]
        span: SourceSpan,
    },
    #[error("unclosed function call")]
    UnclosedFunction {
        #[label("close the function call with `)`")]
        span: SourceSpan,
    },
    #[error("unclosed list literal")]
    UnclosedList {
        #[label("close the list with `]`")]
        span: SourceSpan,
    },
    #[error("unexpected token")]
    UnexpectedToken {
        #[label("remove or correct this token")]
        span: SourceSpan,
    },
    #[error("unexpected token after expression")]
    UnexpectedTokenAfterExpr {
        #[label("remove or correct this token")]
        span: SourceSpan,
    },
    #[error("trailing comma")]
    TrailingComma {
        #[label("remove the trailing comma")]
        span: SourceSpan,
    },
    #[error("empty field name")]
    EmptyField {
        #[label("provide a field name after `.`")]
        span: SourceSpan,
    },
    #[error("stray delimiter")]
    StrayDelimiter {
        #[label("remove or escape this delimiter")]
        span: SourceSpan,
    },
    #[error("`{keyword}` is a reserved keyword")]
    ReservedKeyword {
        keyword: String,
        #[label("rename `{keyword}`")]
        span: SourceSpan,
    },
    #[error("modifier in comment")]
    ModifierInComment {
        #[label("remove the modifier")]
        span: SourceSpan,
    },
    #[error("empty statement")]
    EmptyStatement {
        #[label("provide a statement keyword")]
        span: SourceSpan,
    },
    #[error("unrecognized statement `{stmt}`")]
    UnrecognizedStatement {
        stmt: String,
        #[label("use a valid statement keyword")]
        span: SourceSpan,
    },
    #[error("missing condition")]
    MissingCondition {
        stmt: String,
        #[label("provide a condition after `{stmt}`")]
        span: SourceSpan,
    },
    #[error("empty for loop")]
    EmptyFor {
        #[label("provide a binding like `variable in iterable`")]
        span: SourceSpan,
    },
    #[error("invalid loop variable")]
    InvalidVariable {
        #[label("use a valid identifier")]
        span: SourceSpan,
    },
    #[error("missing `in` keyword")]
    MissingIn {
        #[label("add `in` after the loop variable")]
        span: SourceSpan,
    },
    #[error("missing iterable")]
    MissingIterable {
        #[label("provide an iterable after `in`")]
        span: SourceSpan,
    },
    #[error("unclosed block")]
    UnclosedBlock {
        #[label("add a matching `{{% end %}}`")]
        span: SourceSpan,
    },
    #[error("orphan `end`")]
    OrphanEnd {
        #[label("remove this `end`")]
        span: SourceSpan,
    },
    #[error("`elif` outside of an `if` block")]
    ElifOutsideIf {
        #[label("move `elif` inside an `if` block")]
        span: SourceSpan,
    },
    #[error("`else` outside of an `if` block")]
    ElseOutsideIf {
        #[label("move `else` inside an `if` block")]
        span: SourceSpan,
    },
}

/// Errors raised only on actually-executed content.
#[derive(Debug, Clone, thiserror::Error, Diagnostic, PartialEq)]
pub enum RenderError {
    #[error("undefined variable `{name}`")]
    UndefinedVariable {
        name: String,
        #[label("define this variable or fix the name")]
        span: SourceSpan,
    },
    #[error("map key `{key}` not found")]
    MapKeyNotFound {
        key: String,
        #[label("use an existing key")]
        span: SourceSpan,
    },
    #[error("list index {idx} out of bounds (length {len})")]
    ListIndexOutOfBounds {
        idx: i64,
        len: usize,
        #[label("use an index between 0 and {}", len - 1)]
        span: SourceSpan,
    },
    #[error("negative list index {idx}")]
    NegativeListIndex {
        idx: i64,
        #[label("use a non-negative index")]
        span: SourceSpan,
    },
    #[error("expected {expected}, got {got}")]
    TypeMismatch {
        expected: ValueType,
        got: ValueType,
        #[label("provide a value of type {expected}")]
        span: SourceSpan,
    },
    #[error("undefined function `{name}`")]
    FunctionUndefined {
        name: String,
        #[label("use a defined function name")]
        span: SourceSpan,
    },
    #[error("expected {expected} argument{}, got {got}", if *expected == 1 { "" } else { "s" })]
    FunctionArgCount {
        expected: usize,
        got: usize,
        #[label("{}", if got > expected {
            format!("remove {} argument{}", got - expected, if got - expected == 1 { "" } else { "s" })
        } else {
            format!("add {} argument{}", expected - got, if expected - got == 1 { "" } else { "s" })
        })]
        span: SourceSpan,
    },
}

/// Errors returned by [`FunctionRegistry::call`] — no source spans, used
/// programmatically for matching and conversion to [`RenderError`].
#[derive(Debug, PartialEq)]
pub enum RegistryError {
    Undefined {
        name: String,
    },
    ArgCount {
        expected: usize,
        got: usize,
    },
    TypeMismatch {
        expected: ValueType,
        got: ValueType,
        arg_index: usize,
    },
}
