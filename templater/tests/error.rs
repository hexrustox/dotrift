mod common;

use std::collections::HashMap;

use common::MockRegistry;
use templater::{Error, ParseError, RenderError, Template};
use test_case::test_case;

pub fn parse_err(source: &[u8]) -> (ParseError, (usize, usize)) {
    match Template::from_bytes(source.to_vec()) {
        Err(Error::Parse { err, span }) => (err, (span.offset(), span.len())),
        Err(other) => panic!("expected parse error, got: {other:?}"),
        Ok(_) => panic!("expected parse error, template parsed successfully"),
    }
}

// Scanner errors
#[test_case(b"}}" => (ParseError::StrayDelimiter, (0, 2)) ; "stray_closing_delimiter")]
#[test_case(b"\\{{}}" => (ParseError::StrayDelimiter, (3, 2)) ; "escaped_open_with_unescaped_close")]
#[test_case(b"\\{{ }}" => (ParseError::StrayDelimiter, (4, 2)) ; "escaped_interp_open_then_close")]
#[test_case(b"prefix \\{{ suffix }}" => (ParseError::StrayDelimiter, (18, 2)) ; "with_literal_prefix_and_suffix")]
#[test_case(b"{#= x #}" => (ParseError::InvalidModifier, (2, 1)) ; "modifier_on_comment_open")]
#[test_case(b"{# x =#}" => (ParseError::InvalidModifier, (5, 1)) ; "modifier_on_comment_close")]
// Parser errors
#[test_case(b"{{}}" => (ParseError::EmptyInterpolation, (0, 4)) ; "empty_interpolation_no_padding")]
#[test_case(b"{{   }}" => (ParseError::EmptyInterpolation, (0, 7)) ; "empty_interpolation_with_padding")]
#[test_case(b"{{ 99999999999999999999999 }}" => (ParseError::IntegerOutOfRange, (3, 23)) ; "integer_out_of_range_positive")]
#[test_case(b"{{ -99999999999999999999999 }}" => (ParseError::IntegerOutOfRange, (3, 24)) ; "integer_out_of_range_negative")]
#[test_case(b"{{ +7 }}" => (ParseError::UnexpectedToken, (3, 1)) ; "plus_prefixed_integer")]
#[test_case(b"{{ \"hello }}" => (ParseError::UnclosedString, (3, 9)) ; "unclosed_string_literal")]
#[test_case(b"{{ name" => (ParseError::UnclosedDelimiter, (0, 2)) ; "unclosed_delimiter")]
#[test_case(b"{{ @ }}" => (ParseError::UnexpectedToken, (3, 1)) ; "unexpected_token_at_sign")]
#[test_case(b"{{ a b }}" => (ParseError::UnexpectedTokensAfterExpr, (5, 1)) ; "unexpected_tokens_after_expr")]
#[test_case(b"{{=- x }}" => (ParseError::UnexpectedToken, (3, 1)) ; "left_equal_followed_by_dash")]
#[test_case(b"{{x =-}}" => (ParseError::UnexpectedTokensAfterExpr, (4, 1)) ; "right_dash_after_equal")]
#[test_case(b"{% %}" => (ParseError::UnrecognizedStatement, (0, 5)) ; "empty_statement_no_padding")]
#[test_case(b"{%   %}" => (ParseError::UnrecognizedStatement, (0, 7)) ; "empty_statement_with_padding")]
#[test_case(b"{% if %}" => (ParseError::UnrecognizedStatement, (0, 8)) ; "statement_tag_is_unrecognized")]
#[test_case(b"{% endif %}" => (ParseError::UnrecognizedStatement, (0, 11)) ; "endif_is_unrecognized")]
#[test_case(b"{% endfor %}" => (ParseError::UnrecognizedStatement, (0, 12)) ; "endfor_is_unrecognized")]
#[test_case(b"{% if x" => (ParseError::UnclosedDelimiter, (0, 2)) ; "unclosed_statement")]
#[test_case(b"{# c" => (ParseError::UnclosedDelimiter, (0, 2)) ; "unclosed_comment")]
#[test_case(b"{{ - }}" => (ParseError::UnexpectedToken, (3, 1)) ; "minus_alone")]
#[test_case(b"{{ 1st }}" => (ParseError::UnexpectedTokensAfterExpr, (4, 1)) ; "digit_starting_identifier")]
#[test_case(b"{{ .x }}" => (ParseError::UnexpectedToken, (3, 1)) ; "leading_dot")]
#[test_case(b"{{ \"a\" b }}" => (ParseError::UnexpectedTokensAfterExpr, (7, 1)) ; "trailing_token_after_string")]
#[test_case(b"{{ \"=}}" => (ParseError::UnclosedString, (3, 4)) ; "unclosed_string_with_equal_close")]
#[test_case(b"{{\n}}" => (ParseError::EmptyInterpolation, (0, 5)) ; "newline_only_body")]
fn parse_error_cases(source: &[u8]) -> (ParseError, (usize, usize)) {
    parse_err(source)
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
    let Error::Render {
        err: RenderError::UndefinedVariable,
        ..
    } = e
    else {
        panic!("expected undefined variable because `if` is parsed as a variable, got: {e:?}");
    };
}

// --- Render errors ------------------------------------------------------

/// A writer that always fails on `write`, used to verify IO errors propagate.
struct FailingWriter;

impl std::io::Write for FailingWriter {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("write failure"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn render_propagates_io_error() {
    let template = Template::from_bytes(b"hello {{ name }}!".to_vec()).expect("parse failed");
    let e = template
        .render(&mut FailingWriter, &HashMap::new(), &MockRegistry)
        .unwrap_err();
    assert!(matches!(e, Error::Io(_)), "expected IO error, got: {e:?}");
}

#[test_case(b"hi {{ nope }}!" => (6, 4) ; "undefined_var_mid_source")]
#[test_case(b"{{ missing }}" => (3, 7) ; "undefined_var_at_start_of_source")]
fn undefined_variable_carries_name_span(source: &[u8]) -> (usize, usize) {
    let template = Template::from_bytes(source.to_vec()).expect("parse failed");
    let mut out = Vec::new();
    let e = template
        .render(&mut out, &HashMap::new(), &MockRegistry)
        .unwrap_err();
    let Error::Render { err: r, span } = e else {
        panic!("expected render error, got: {e:?}");
    };
    assert!(matches!(r, RenderError::UndefinedVariable));
    (span.offset(), span.len())
}
