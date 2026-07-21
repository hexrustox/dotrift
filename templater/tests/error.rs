mod common;

use std::collections::HashMap;

use common::{MockRegistry, TestRegistry};
use templater::{Error, FuncError, ParseError, RenderError, Template, ValueType};
use test_case::test_case;

use crate::common::var_scope;

#[test_case(b"}}" => (ParseError::StrayDelimiter, (0, 2)) ; "stray_closing_delimiter")]
#[test_case(b"\\{{}}" => (ParseError::StrayDelimiter, (3, 2)) ; "escaped_open_with_unescaped_close")]
#[test_case(b"\\{{ }}" => (ParseError::StrayDelimiter, (4, 2)) ; "escaped_interp_open_then_close")]
#[test_case(b"prefix \\{{ suffix }}" => (ParseError::StrayDelimiter, (18, 2)) ; "with_literal_prefix_and_suffix")]
#[test_case(b"{#= x #}" => (ParseError::InvalidModifier, (2, 1)) ; "modifier_on_comment_open")]
#[test_case(b"{# x =#}" => (ParseError::InvalidModifier, (5, 1)) ; "modifier_on_comment_close")]
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
fn parse_error(source: &[u8]) -> (ParseError, (usize, usize)) {
    match Template::from_bytes(source.to_vec()) {
        Err(Error::Parse { err, span }) => (err, (span.offset(), span.len())),
        _ => panic!("expected parse error"),
    }
}

#[test_case(b"{{ nope }}!" => matches (RenderError::UndefinedVariable, (3, 4)) ; "undefined_var_mid_source")]
#[test_case(b"{{ missing }}" => matches (RenderError::UndefinedVariable, (3, 7)) ; "undefined_var_at_start_of_source")]
#[test_case(b"{{ items.name }}" => matches (RenderError::MapAccessOnNonMap { got: ValueType::List }, (9, 4)) ; "dot_on_list")]
#[test_case(b"{{ user.0 }}" => matches (RenderError::ListAccessOnNonList { got: ValueType::Map }, (8, 1)) ; "index_on_map")]
#[test_case(b"{{ user.name.0 }}" => matches (RenderError::ListAccessOnNonList { got: ValueType::Str }, (13, 1)) ; "index_on_string")]
#[test_case(b"{{ user.name.field }}" => matches (RenderError::MapAccessOnNonMap { got: ValueType::Str }, (13, 5)) ; "dot_on_string")]
#[test_case(b"{{ items.-1 }}" => matches (RenderError::NegativeListIndex { idx: -1 }, (9, 2)) ; "negative_index")]
#[test_case(b"{{ items.5 }}" => matches (RenderError::ListIndexOutOfBounds { idx: 5, len: 3 }, (9, 1)) ; "index_out_of_bounds")]
#[test_case(b"{{ user.missing }}" => matches (RenderError::MapKeyNotFound { key }, (8, 7)) if key == "missing" ; "map_key_not_found")]
#[test_case(b"{{ count.0 }}" => matches (RenderError::ListAccessOnNonList { got: ValueType::Int }, (9, 1)) ; "index_on_int")]
#[test_case(b"{{ flag.0 }}" => matches (RenderError::ListAccessOnNonList { got: ValueType::Bool }, (8, 1)) ; "index_on_bool")]
#[test_case(b"{{ count.name }}" => matches (RenderError::MapAccessOnNonMap { got: ValueType::Int }, (9, 4)) ; "dot_on_int")]
#[test_case(b"{{ flag.name }}" => matches (RenderError::MapAccessOnNonMap { got: ValueType::Bool }, (8, 4)) ; "dot_on_bool")]
fn render_error(source: &[u8]) -> (RenderError, (usize, usize)) {
    let template = Template::from_bytes(source.to_vec()).expect("parse failed");
    let mut out = Vec::new();
    let e = template
        .render(&mut out, &var_scope(), &MockRegistry)
        .unwrap_err();
    let Error::Render { err, span } = e else {
        panic!("expected render error");
    };
    (err, (span.offset(), span.len()))
}

#[test_case(b"{{ missing() }}" => matches (FuncError::Undefined { name }, (3, 7)) if name == "missing" ; "undefined_function")]
#[test_case(b"{{ not() }}" => matches (FuncError::ArgCount { expected: 1, got: 0 }, (6, 2)) ; "arg_count_zero_args")]
#[test_case(b"{{ length(items, items) }}" => matches (FuncError::ArgCount { expected: 1, got: 2 }, (10, 12)) ; "arg_count_too_many")]
#[test_case(b"{{ not(1) }}" => matches (FuncError::TypeMismatch { expected: ValueType::Bool, got: ValueType::Int, arg_index: 0 }, (7, 1)) ; "type_mismatch_arg1")]
#[test_case(br#"{{ not(" ") }}"# => matches (FuncError::TypeMismatch { expected: ValueType::Bool, got: ValueType::Str, arg_index: 0 }, (7, 3)) ; "type_mismatch_arg2")]
#[test_case(br#"{{ not(length(items)) }}"# => matches (FuncError::TypeMismatch { expected: ValueType::Bool, got: ValueType::Int, arg_index: 0 }, (7, 13)) ; "type_mismatch_arg3")]
#[test_case(b"{{ not([1]) }}" => matches (FuncError::TypeMismatch { expected: ValueType::Bool, got: ValueType::List, arg_index: 0 }, (7, 3)) ; "type_mismatch_arg4")]
#[test_case(b"{{ not([ 1, 2 ]) }}" => matches (FuncError::TypeMismatch { expected: ValueType::Bool, got: ValueType::List, arg_index: 0 }, (7, 8)) ; "type_mismatch_arg7")]
#[test_case(b"{{ not(user.name) }}" => matches (FuncError::TypeMismatch { expected: ValueType::Bool, got: ValueType::Str, arg_index: 0 }, (7, 9)) ; "type_mismatch_arg5")]
#[test_case(b"{{ not(items.0) }}" => matches (FuncError::TypeMismatch { expected: ValueType::Bool, got: ValueType::Str, arg_index: 0 }, (7, 7)) ; "type_mismatch_arg6")]
fn func_error(source: &[u8]) -> (FuncError, (usize, usize)) {
    let template = Template::from_bytes(source.to_vec()).expect("parse failed");
    let mut out = Vec::new();
    let e = template
        .render(&mut out, &var_scope(), &TestRegistry)
        .unwrap_err();
    let Error::Func { err, span } = e else {
        panic!("expected func error");
    };
    (err, (span.offset(), span.len()))
}

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
    let template = Template::from_bytes(b"hello".to_vec()).expect("parse failed");
    let e = template
        .render(&mut FailingWriter, &HashMap::new(), &MockRegistry)
        .unwrap_err();
    assert!(matches!(e, Error::Io(_)), "expected IO error, got: {e:?}");
}

struct FlushFailingWriter;

impl std::io::Write for FlushFailingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::other("flush failure"))
    }
}

#[test]
fn render_propagates_flush_error() {
    let template = Template::from_bytes(b"".to_vec()).expect("parse failed");
    let e = template
        .render(&mut FlushFailingWriter, &var_scope(), &MockRegistry)
        .unwrap_err();
    assert!(matches!(e, Error::Io(_)), "expected IO error, got: {e:?}");
}
