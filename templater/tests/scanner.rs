mod common;

use std::collections::HashMap;

use common::{MockRegistry, render};
use templater::{Error, ParseError, Template, Value};

fn var_scope() -> HashMap<String, Value> {
    HashMap::from([("name".to_string(), Value::Str("world".to_string()))])
}

fn render_err(source: &[u8]) -> Error {
    match Template::from_bytes(source.to_vec()) {
        Err(e) => e,
        Ok(_) => panic!("expected parse error"),
    }
}

// --- Escape rule (rendered through the public API) -------------------------

#[test]
fn escaped_interp_open_renders_literal_braces() {
    let out = render(b"before \\{{ after", &var_scope(), &MockRegistry);
    assert_eq!(out, b"before {{ after");
}

#[test]
fn escaped_interp_open_and_close_renders_literal_braces() {
    let out = render(b"\\{{\\}}", &var_scope(), &MockRegistry);
    assert_eq!(out, b"{{}}");
}

#[test]
fn two_backslashes_render_one_and_keep_tag_active() {
    let out = render(b"\\\\{{ name }}", &var_scope(), &MockRegistry);
    assert_eq!(out, b"\\world");
}

#[test]
fn three_backslashes_render_one_and_escape_tag() {
    let out = render(b"\\\\\\{{", &var_scope(), &MockRegistry);
    assert_eq!(out, b"\\{{");
}

#[test]
fn four_backslashes_render_two_and_keep_tag_active() {
    let out = render(b"\\\\\\\\{{ name }}", &var_scope(), &MockRegistry);
    assert_eq!(out, b"\\\\world");
}

// --- Comments -------------------------------------------------------------

#[test]
fn comment_is_stripped() {
    let out = render(b"{# secret #}visible", &HashMap::new(), &MockRegistry);
    assert_eq!(out, b"visible");
}

#[test]
fn comment_splits_plain_text() {
    let out = render(b"before {# c #} after", &HashMap::new(), &MockRegistry);
    assert_eq!(out, b"before  after");
}

// --- Dash whitespace modifier ---------------------------------------------

#[test]
fn left_dash_trims_adjacent_spaces() {
    let out = render(b"  {{- name }}", &var_scope(), &MockRegistry);
    assert_eq!(out, b"world");
}

#[test]
fn right_dash_trims_adjacent_spaces() {
    let out = render(b"{{ name -}}  ", &var_scope(), &MockRegistry);
    assert_eq!(out, b"world");
}

#[test]
fn dash_does_not_trim_newline() {
    let out = render(b"before\n  {{- name }}", &var_scope(), &MockRegistry);
    assert_eq!(out, b"before\nworld");
}

// --- Equal whitespace modifier --------------------------------------------

#[test]
fn left_equal_eats_to_line_start() {
    let out = render(b"prefix {{= name }}", &var_scope(), &MockRegistry);
    assert_eq!(out, b"world");
}

#[test]
fn right_equal_eats_through_newline() {
    let out = render(b"{{ name =}}suffix\nnext", &var_scope(), &MockRegistry);
    assert_eq!(out, b"worldnext");
}

#[test]
fn equal_tags_share_a_line_delete_between() {
    let out = render(b"{{ name =}} mid {{= name }}", &var_scope(), &MockRegistry);
    assert_eq!(out, b"worldworld");
}

#[test]
fn equal_respects_comment_barrier() {
    let out = render(
        b"{{ name =}} {# c #} {{= name }}",
        &var_scope(),
        &MockRegistry,
    );
    assert_eq!(out, b"worldworld");
}

#[test]
fn equal_stops_before_newline_left() {
    let out = render(b"keep\nremove {{= name }}", &var_scope(), &MockRegistry);
    assert_eq!(out, b"keep\nworld");
}

#[test]
fn equal_eats_cr_as_plain_text() {
    // \r is not a line terminator, so left `=` stops at the real \n and the
    // \r and the \n both survive.
    let out = render(b"a\r\nkeep {{= name }}", &var_scope(), &MockRegistry);
    assert_eq!(out, b"a\r\nworld");
}

// --- Scanner errors --------------------------------------------------------

#[test]
fn stray_closing_delimiter() {
    let e = render_err(b"}}").unwrap_parse();
    assert!(matches!(e, ParseError::StrayDelimiter { .. }));
}

#[test]
fn escaped_open_with_unescaped_close_errors() {
    let e = render_err(b"\\{{}}").unwrap_parse();
    assert!(matches!(e, ParseError::StrayDelimiter { .. }));
}

#[test]
fn statement_tag_is_unrecognized() {
    let e = render_err(b"{% if %}").unwrap_parse();
    assert!(matches!(e, ParseError::UnrecognizedStatement { .. }));
}

#[test]
fn modifier_on_comment_open_errors() {
    let e = render_err(b"{#= x #}").unwrap_parse();
    assert!(matches!(e, ParseError::InvalidModifier { .. }));
}

#[test]
fn modifier_on_comment_close_errors() {
    let e = render_err(b"{# x =#}").unwrap_parse();
    assert!(matches!(e, ParseError::InvalidModifier { .. }));
}

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
