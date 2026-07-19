mod common;

use std::collections::HashMap;

use common::MockRegistry;
use templater::{Error, ParseError, RenderError, Template};

pub fn parse_err(source: &[u8]) -> ParseError {
    match Template::from_bytes(source.to_vec()) {
        Err(Error::Parse(e)) => e,
        Err(other) => panic!("expected parse error, got: {other:?}"),
        Ok(_) => panic!("expected parse error, template parsed successfully"),
    }
}

// --- Scanner errors -------------------------------------------------------

#[test]
fn stray_closing_delimiter() {
    let e = parse_err(b"}}");
    assert!(matches!(e, ParseError::StrayDelimiter { .. }));
    assert_eq!(e.span(), (0, 2));
}

#[test]
fn escaped_open_with_unescaped_close_errors() {
    let e = parse_err(b"\\{{}}");
    assert!(matches!(e, ParseError::StrayDelimiter { .. }));
    assert_eq!(e.span(), (3, 2));
}

#[test]
fn statement_tag_is_unrecognized() {
    let e = parse_err(b"{% if %}");
    assert!(matches!(e, ParseError::UnrecognizedStatement { .. }));
    assert_eq!(e.span(), (0, 8));
}

#[test]
fn modifier_on_comment_open_errors() {
    let e = parse_err(b"{#= x #}");
    assert!(matches!(e, ParseError::InvalidModifier { .. }));
    assert_eq!(e.span(), (2, 1));
}

#[test]
fn modifier_on_comment_close_errors() {
    let e = parse_err(b"{# x =#}");
    assert!(matches!(e, ParseError::InvalidModifier { .. }));
    assert_eq!(e.span(), (5, 1));
}

// --- Parse errors --------------------------------------------------------

#[test]
fn empty_interpolation_no_padding() {
    let e = parse_err(b"{{}}");
    assert!(matches!(e, ParseError::EmptyInterpolation { .. }));
    assert_eq!(e.span(), (0, 4));
}

#[test]
fn empty_interpolation_with_padding() {
    let e = parse_err(b"{{   }}");
    assert!(matches!(e, ParseError::EmptyInterpolation { .. }));
    assert_eq!(e.span(), (0, 7));
}

#[test]
fn integer_out_of_range_positive() {
    let e = parse_err(b"{{ 99999999999999999999999 }}");
    assert!(matches!(e, ParseError::IntegerOutOfRange { .. }));
    // span covers the integer digits only
    assert_eq!(e.span(), (3, 23));
}

#[test]
fn integer_out_of_range_negative() {
    let e = parse_err(b"{{ -99999999999999999999999 }}");
    assert!(matches!(e, ParseError::IntegerOutOfRange { .. }));
    assert_eq!(e.span(), (3, 24));
}

#[test]
fn plus_prefixed_integer() {
    let e = parse_err(b"{{ +7 }}");
    assert!(matches!(e, ParseError::UnexpectedToken { .. }));
    assert_eq!(e.span(), (3, 1));
}

#[test]
fn unclosed_string_literal() {
    let e = parse_err(b"{{ \"hello }}");
    assert!(matches!(e, ParseError::UnclosedString { .. }));
    // span covers the opening `"` through end of source
    assert_eq!(e.span(), (3, 9));
}

#[test]
fn unclosed_delimiter() {
    let e = parse_err(b"{{ name");
    assert!(matches!(e, ParseError::UnclosedDelimiter { .. }));
    assert_eq!(e.span(), (0, 2));
}

#[test]
fn unexpected_token_at_sign() {
    let e = parse_err(b"{{ @ }}");
    assert!(matches!(e, ParseError::UnexpectedToken { .. }));
    assert_eq!(e.span(), (3, 1));
}

#[test]
fn unexpected_tokens_after_expr() {
    let e = parse_err(b"{{ a b }}");
    assert!(matches!(e, ParseError::UnexpectedTokensAfterExpr { .. }));
    assert_eq!(e.span(), (5, 1));
}

#[test]
fn left_equal_followed_by_dash() {
    let e = parse_err(b"{{=- x }}");
    assert!(matches!(e, ParseError::UnexpectedToken { .. }));
    assert_eq!(e.span(), (3, 1));
}

#[test]
fn right_dash_after_equal() {
    let e = parse_err(b"{{x =-}}");
    assert!(matches!(e, ParseError::UnexpectedTokensAfterExpr { .. }));
    assert_eq!(e.span(), (4, 1));
}

// --- Render errors ------------------------------------------------------

#[test]
fn undefined_variable_carries_name_span() {
    let template = Template::from_bytes(b"hi {{ nope }}!".to_vec()).expect("parse failed");
    let mut out = Vec::new();
    let e = template
        .render(&mut out, &HashMap::new(), &MockRegistry)
        .unwrap_err();
    let Error::Render(r) = e else {
        panic!("expected render error, got: {e:?}");
    };
    assert!(matches!(r, RenderError::UndefinedVariable { .. }));
    assert_eq!(r.span(), (6, 4));
}
