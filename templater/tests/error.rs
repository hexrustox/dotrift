mod common;

use std::collections::HashMap;

use common::{MockRegistry, var_scope};
use templater::{Error, Template, util::TestRegistry};
use test_case::test_case;

// scanner parse errors
#[test_case(b"{{ x"; "unclosed_interpolation")]
#[test_case(b"{% if x"; "unclosed_statement")]
#[test_case(b"{# comment"; "unclosed_comment")]
#[test_case(b"}}"; "stray_delimiter_interp")]
#[test_case(b"%}"; "stray_delimiter_stmt")]
#[test_case(b"#}"; "stray_delimiter_comment")]
#[test_case(b"{#- x #}"; "invalid_modifier_on_comment")]
#[test_case(b"{{ x \\-}}"; "escaped_modifier_close_unclosed")]
// expression parse errors
#[test_case(b"{{ }}"; "empty_interpolation")]
#[test_case(b"{% %}"; "empty_statement")]
#[test_case(b"{{ 9223372036854775808 }}"; "integer_overflow")]
#[test_case(b"{{ \"unterminated }}"; "unclosed_string")]
#[test_case(b"{{ x. }}"; "trailing_dot")]
#[test_case(b"{{ if }}"; "keyword_as_identifier")]
#[test_case(b"{{ [1,] }}"; "trailing_comma_list")]
#[test_case(b"{{ same(1,) }}"; "trailing_comma_call")]
#[test_case(b"{{ same(1 }}"; "unclosed_function")]
#[test_case(b"{{ @ }}"; "unexpected_token")]
#[test_case(b"{{ a b }}"; "unexpected_token_after_expr")]
// statement / block parse errors
#[test_case(b"{% endif %}"; "unrecognized_statement")]
#[test_case(b"{% end %}"; "orphan_end")]
#[test_case(b"{% elif x %}"; "orphan_elif")]
#[test_case(b"{% else %}"; "orphan_else")]
#[test_case(b"{% if %}"; "missing_condition")]
#[test_case(b"{% for %}"; "empty_for")]
#[test_case(b"{% for $ %}"; "invalid_variable")]
#[test_case(b"{% for x %}"; "missing_in")]
#[test_case(b"{% for x in %}"; "missing_iterable")]
#[test_case(b"{% if true %}hi"; "unclosed_if_block")]
#[test_case(b"{% for x in list %}hi"; "unclosed_for_block")]
#[test_case(b"{% if true %}{% else x %}{% end %}"; "else_with_operand")]
// render-time errors
#[test_case(b"{{ missing }}"; "undefined_variable")]
#[test_case(b"{{ map.nope }}"; "map_key_not_found")]
#[test_case(b"{{ list.5 }}"; "list_index_out_of_bounds")]
#[test_case(b"{{ list.-1 }}"; "negative_list_index")]
#[test_case(b"{% if str %}x{% end %}"; "if_cond_type_mismatch_str")]
#[test_case(b"{% for x in str %}x{% end %}"; "for_iterable_type_mismatch_str")]
#[test_case(b"{{ str.field }}"; "dot_access_on_non_map")]
#[test_case(b"{{ str.0 }}"; "index_access_on_non_list")]
// function errors
#[test_case(b"{{ nope() }}"; "undefined_function")]
#[test_case(b"{{ one_arg() }}"; "arg_count_zero")]
#[test_case(b"{{ one_arg(1, 2) }}"; "arg_count_two")]
#[test_case(b"{{ two_arg(1) }}"; "arg_count_one")]
#[test_case(b"{{ mismatch(42) }}"; "type_mismatch_arg")]
// span / source-window edge cases
#[test_case(b"hello {{  }}"; "empty_interp_after_text")]
#[test_case(b"line1\nline2\n{{  }}"; "empty_interp_on_later_line")]
#[test_case(b"{{\n  @\n}}"; "unexpected_token_multiline_body")]
fn debug_render_report(src: &[u8]) {
    let _ = miette::set_hook(Box::new(|_| {
        Box::new(
            miette::MietteHandlerOpts::new()
                .unicode(false)
                .color(false)
                .build(),
        )
    }));

    let template = Template::from_bytes(src);
    let result = template.render(&mut Vec::new(), &var_scope(), &TestRegistry);
    let report = template.report(result).unwrap_err();
    insta::assert_snapshot!(
        std::thread::current().name().unwrap().replace(":", "_"),
        format!("{report:?}")
    );
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
    let err = Template::from_bytes(b"hello")
        .render(&mut FailingWriter, &HashMap::new(), &MockRegistry)
        .unwrap_err();
    assert!(
        matches!(err, Error::Io(_)),
        "expected IO error, got: {err:?}"
    );
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
    let err = Template::from_bytes(b"")
        .render(&mut FlushFailingWriter, &var_scope(), &MockRegistry)
        .unwrap_err();
    assert!(
        matches!(err, Error::Io(_)),
        "expected IO error, got: {err:?}"
    );
}
