mod common;

use std::collections::HashMap;

use common::MockRegistry;
use templater::{Error, ParseError, RenderError, Template};
use test_case::test_case;

pub fn parse_err(source: &[u8]) -> ParseError {
    match Template::from_bytes(source.to_vec()) {
        Err(Error::Parse(e)) => e,
        Err(other) => panic!("expected parse error, got: {other:?}"),
        Ok(_) => panic!("expected parse error, template parsed successfully"),
    }
}

/// A stable stand-in for `ParseError` equality checks in `#[test_case]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseErrorKind {
    EmptyInterpolation,
    IntegerOutOfRange,
    UnclosedString,
    UnclosedDelimiter,
    UnexpectedToken,
    UnexpectedTokensAfterExpr,
    StrayDelimiter,
    InvalidModifier,
    UnrecognizedStatement,
}

impl From<&ParseError> for ParseErrorKind {
    fn from(e: &ParseError) -> Self {
        match e {
            ParseError::EmptyInterpolation { .. } => Self::EmptyInterpolation,
            ParseError::IntegerOutOfRange { .. } => Self::IntegerOutOfRange,
            ParseError::UnclosedString { .. } => Self::UnclosedString,
            ParseError::UnclosedDelimiter { .. } => Self::UnclosedDelimiter,
            ParseError::UnexpectedToken { .. } => Self::UnexpectedToken,
            ParseError::UnexpectedTokensAfterExpr { .. } => Self::UnexpectedTokensAfterExpr,
            ParseError::StrayDelimiter { .. } => Self::StrayDelimiter,
            ParseError::InvalidModifier { .. } => Self::InvalidModifier,
            ParseError::UnrecognizedStatement { .. } => Self::UnrecognizedStatement,
        }
    }
}

// --- Scanner errors -------------------------------------------------------

#[test_case(b"}}" => (ParseErrorKind::StrayDelimiter, (0, 2)) ; "stray_closing_delimiter")]
#[test_case(b"\\{{}}" => (ParseErrorKind::StrayDelimiter, (3, 2)) ; "escaped_open_with_unescaped_close")]
#[test_case(b"{#= x #}" => (ParseErrorKind::InvalidModifier, (2, 1)) ; "modifier_on_comment_open")]
#[test_case(b"{# x =#}" => (ParseErrorKind::InvalidModifier, (5, 1)) ; "modifier_on_comment_close")]
fn scanner_error_cases(source: &[u8]) -> (ParseErrorKind, (usize, usize)) {
    let e = parse_err(source);
    (ParseErrorKind::from(&e), e.span())
}

// --- Parse errors --------------------------------------------------------

#[test_case(b"{{}}" => (ParseErrorKind::EmptyInterpolation, (0, 4)) ; "empty_interpolation_no_padding")]
#[test_case(b"{{   }}" => (ParseErrorKind::EmptyInterpolation, (0, 7)) ; "empty_interpolation_with_padding")]
// Span covers the integer digits only.
#[test_case(b"{{ 99999999999999999999999 }}" => (ParseErrorKind::IntegerOutOfRange, (3, 23)) ; "integer_out_of_range_positive")]
#[test_case(b"{{ -99999999999999999999999 }}" => (ParseErrorKind::IntegerOutOfRange, (3, 24)) ; "integer_out_of_range_negative")]
#[test_case(b"{{ +7 }}" => (ParseErrorKind::UnexpectedToken, (3, 1)) ; "plus_prefixed_integer")]
// Span covers the opening `"` through end of source.
#[test_case(b"{{ \"hello }}" => (ParseErrorKind::UnclosedString, (3, 9)) ; "unclosed_string_literal")]
#[test_case(b"{{ name" => (ParseErrorKind::UnclosedDelimiter, (0, 2)) ; "unclosed_delimiter")]
#[test_case(b"{{ @ }}" => (ParseErrorKind::UnexpectedToken, (3, 1)) ; "unexpected_token_at_sign")]
#[test_case(b"{{ a b }}" => (ParseErrorKind::UnexpectedTokensAfterExpr, (5, 1)) ; "unexpected_tokens_after_expr")]
#[test_case(b"{{=- x }}" => (ParseErrorKind::UnexpectedToken, (3, 1)) ; "left_equal_followed_by_dash")]
#[test_case(b"{{x =-}}" => (ParseErrorKind::UnexpectedTokensAfterExpr, (4, 1)) ; "right_dash_after_equal")]
#[test_case(b"{% %}" => (ParseErrorKind::UnrecognizedStatement, (0, 5)) ; "empty_statement_no_padding")]
#[test_case(b"{%   %}" => (ParseErrorKind::UnrecognizedStatement, (0, 7)) ; "empty_statement_with_padding")]
#[test_case(b"{% if %}" => (ParseErrorKind::UnrecognizedStatement, (0, 8)) ; "statement_tag_is_unrecognized")]
#[test_case(b"{% endif %}" => (ParseErrorKind::UnrecognizedStatement, (0, 11)) ; "endif_is_unrecognized")]
#[test_case(b"{% endfor %}" => (ParseErrorKind::UnrecognizedStatement, (0, 12)) ; "endfor_is_unrecognized")]
#[test_case(b"{% if x" => (ParseErrorKind::UnclosedDelimiter, (0, 2)) ; "unclosed_statement")]
#[test_case(b"{# c" => (ParseErrorKind::UnclosedDelimiter, (0, 2)) ; "unclosed_comment")]
#[test_case(b"{{ - }}" => (ParseErrorKind::UnexpectedToken, (3, 1)) ; "minus_alone")]
#[test_case(b"{{ 1st }}" => (ParseErrorKind::UnexpectedTokensAfterExpr, (4, 1)) ; "digit_starting_identifier")]
#[test_case(b"{{ .x }}" => (ParseErrorKind::UnexpectedToken, (3, 1)) ; "leading_dot")]
#[test_case(b"{{ \"a\" b }}" => (ParseErrorKind::UnexpectedTokensAfterExpr, (7, 1)) ; "trailing_token_after_string")]
fn parse_error_cases(source: &[u8]) -> (ParseErrorKind, (usize, usize)) {
    let e = parse_err(source);
    (ParseErrorKind::from(&e), e.span())
}

// --- Spec mismatch: reserved keywords as identifiers --------------------
//
// The spec says `if`, `elif`, `else`, `for`, `in`, and `end` are reserved
// keywords and cannot be used as variable names. Today the parser treats
// `{{ if }}` as a variable reference, so it is *not* a parse error. This
// test documents the current behavior; it will fail once the parser is
// updated to reject keywords as identifiers.

#[test]
fn keyword_as_identifier_is_currently_accepted() {
    let template = Template::from_bytes(b"{{ if }}".to_vec()).expect("parse failed");
    let mut out = Vec::new();
    let e = template
        .render(&mut out, &HashMap::new(), &MockRegistry)
        .unwrap_err();
    let Error::Render(RenderError::UndefinedVariable { .. }) = e else {
        panic!("expected undefined variable because `if` is parsed as a variable, got: {e:?}");
    };
}

// --- Render errors ------------------------------------------------------

#[test_case(b"hi {{ nope }}!" => (6, 4) ; "mid_source")]
#[test_case(b"{{ missing }}" => (3, 7) ; "at_start_of_source")]
fn undefined_variable_carries_name_span(source: &[u8]) -> (usize, usize) {
    let template = Template::from_bytes(source.to_vec()).expect("parse failed");
    let mut out = Vec::new();
    let e = template
        .render(&mut out, &HashMap::new(), &MockRegistry)
        .unwrap_err();
    let Error::Render(r) = e else {
        panic!("expected render error, got: {e:?}");
    };
    assert!(matches!(r, RenderError::UndefinedVariable { .. }));
    r.span()
}
