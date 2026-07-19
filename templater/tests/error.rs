mod common;

use std::collections::HashMap;

use common::MockRegistry;
use templater::{Error, ParseError, RenderError, Template, Value};

/// Parse an error template + attempt render, returning the underlying `Error`.
fn render_err(source: &[u8]) -> Error {
    let template = match Template::from_bytes(source.to_vec()) {
        Ok(t) => t,
        Err(e) => return e,
    };
    let mut out = Vec::new();
    match template.render(&mut out, &HashMap::new(), &MockRegistry) {
        Ok(()) => panic!("expected error, rendered: {out:?}"),
        Err(e) => e,
    }
}

/// Extract `(kind, span_offset, span_len)` from a parse error.
fn parse_span(e: &ParseError) -> (usize, usize) {
    match e {
        ParseError::EmptyInterpolation { span }
        | ParseError::IntegerOutOfRange { span }
        | ParseError::UnclosedString { span }
        | ParseError::UnclosedDelimiter { span }
        | ParseError::UnexpectedToken { span }
        | ParseError::UnexpectedTokensAfterExpr { span }
        | ParseError::StrayDelimiter { span }
        | ParseError::InvalidModifier { span }
        | ParseError::UnrecognizedStatement { span } => (span.offset(), span.len()),
    }
}

fn render_span(e: &RenderError) -> (usize, usize) {
    match e {
        RenderError::UndefinedVariable { span } => (span.offset(), span.len()),
    }
}

// --- Parse errors --------------------------------------------------------

#[test]
fn empty_interpolation_no_padding() {
    let e = render_err(b"{{}}").unwrap_parse();
    assert!(matches!(e, ParseError::EmptyInterpolation { .. }));
    assert_eq!(parse_span(&e), (0, 4));
}

#[test]
fn empty_interpolation_with_padding() {
    let e = render_err(b"{{   }}").unwrap_parse();
    assert!(matches!(e, ParseError::EmptyInterpolation { .. }));
    assert_eq!(parse_span(&e), (0, 7));
}

#[test]
fn integer_out_of_range_positive() {
    let e = render_err(b"{{ 99999999999999999999999 }}").unwrap_parse();
    assert!(matches!(e, ParseError::IntegerOutOfRange { .. }));
    // span covers the integer digits only
    assert_eq!(parse_span(&e), (3, 23));
}

#[test]
fn integer_out_of_range_negative() {
    let e = render_err(b"{{ -99999999999999999999999 }}").unwrap_parse();
    assert!(matches!(e, ParseError::IntegerOutOfRange { .. }));
    assert_eq!(parse_span(&e), (3, 24));
}

#[test]
fn plus_prefixed_integer() {
    let e = render_err(b"{{ +7 }}").unwrap_parse();
    assert!(matches!(e, ParseError::UnexpectedToken { .. }));
    assert_eq!(parse_span(&e), (3, 1));
}

#[test]
fn unclosed_string_literal() {
    let e = render_err(b"{{ \"hello }}").unwrap_parse();
    assert!(matches!(e, ParseError::UnclosedString { .. }));
    // span covers the opening `"` through end of source
    assert_eq!(parse_span(&e), (3, 9));
}

#[test]
fn unclosed_delimiter() {
    let e = render_err(b"{{ name").unwrap_parse();
    assert!(matches!(e, ParseError::UnclosedDelimiter { .. }));
    assert_eq!(parse_span(&e), (0, 2));
}

#[test]
fn unexpected_token_at_sign() {
    let e = render_err(b"{{ @ }}").unwrap_parse();
    assert!(matches!(e, ParseError::UnexpectedToken { .. }));
    assert_eq!(parse_span(&e), (3, 1));
}

#[test]
fn unexpected_tokens_after_expr() {
    let e = render_err(b"{{ a b }}").unwrap_parse();
    assert!(matches!(e, ParseError::UnexpectedTokensAfterExpr { .. }));
    assert_eq!(parse_span(&e), (5, 1));
}

#[test]
fn left_equal_followed_by_dash() {
    let e = render_err(b"{{=- x }}").unwrap_parse();
    assert!(matches!(e, ParseError::UnexpectedToken { .. }));
    assert_eq!(parse_span(&e), (3, 1));
}

#[test]
fn right_dash_after_equal() {
    let e = render_err(b"{{x =-}}").unwrap_parse();
    assert!(matches!(e, ParseError::UnexpectedTokensAfterExpr { .. }));
    assert_eq!(parse_span(&e), (4, 1));
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
    assert_eq!(render_span(&r), (6, 4));
}

#[test]
fn undefined_variable_in_never_taken_branch_does_not_fire() {
    // Render-time errors fire only on executed content. This ticket has no
    // control-flow statements yet, so we smoke-test that interpolating a
    // missing variable via a literal-only template renders fine.
    let out = common::render(b"{{ 42 }}", &HashMap::new(), &MockRegistry);
    assert_eq!(out, b"42");
}

// Convenience helper to unwrap a parse error from `Error`.
trait UnwrapParse {
    fn unwrap_parse(self) -> ParseError;
}

impl UnwrapParse for Error {
    fn unwrap_parse(self) -> ParseError {
        match self {
            Error::Parse(e) => e,
            other => panic!("expected parse error, got: {other:?}"),
        }
    }
}

// Keep `Value` reachable for future expansion.
#[test]
fn value_in_scope_does_not_pollute_tests() {
    let _ = Value::Int(0);
}
