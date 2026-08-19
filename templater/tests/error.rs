mod common;

use std::collections::HashMap;

use templater::{Template, error::Error, util::TestRegistry};
use test_case::test_case;

use common::{MockRegistry, var_scope};

// scanner parse errors
#[test_case(b"{{ x"; "reports_unclosed_interpolation")]
#[test_case(b"{% if x"; "reports_unclosed_statement")]
#[test_case(b"{# comment"; "reports_unclosed_comment")]
#[test_case(b"}}"; "reports_stray_interpolation_delimiter")]
#[test_case(b"%}"; "reports_stray_statement_delimiter")]
#[test_case(b"#}"; "reports_stray_comment_delimiter")]
#[test_case(b"{#- x #}"; "reports_invalid_trim_modifier_on_comment")]
#[test_case(b"{{ x \\-}}"; "reports_escaped_modifier_close_unclosed")]
// expression parse errors
#[test_case(b"{{ }}"; "reports_empty_interpolation")]
#[test_case(b"{% %}"; "reports_empty_statement")]
#[test_case(b"{{ 9223372036854775808 }}"; "reports_integer_overflow")]
#[test_case(b"{{ \"unterminated }}"; "reports_unclosed_string_literal")]
#[test_case(b"{{ x. }}"; "reports_trailing_dot_in_access")]
#[test_case(b"{{ if }}"; "reports_reserved_keyword_as_identifier")]
#[test_case(b"{{ [1,] }}"; "reports_trailing_comma_in_list")]
#[test_case(b"{{ same(1,) }}"; "reports_trailing_comma_in_call")]
#[test_case(b"{{ same(1 }}"; "reports_unclosed_function_call")]
#[test_case(b"{{ @ }}"; "reports_unexpected_token")]
#[test_case(b"{{ a b }}"; "reports_token_after_complete_expr")]
// statement / block parse errors
#[test_case(b"{% endif %}"; "reports_unrecognized_statement")]
#[test_case(b"{% end %}"; "reports_orphan_end")]
#[test_case(b"{% elif x %}"; "reports_orphan_elif")]
#[test_case(b"{% else %}"; "reports_orphan_else")]
#[test_case(b"{% if %}"; "reports_missing_if_condition")]
#[test_case(b"{% for %}"; "reports_empty_for_loop")]
#[test_case(b"{% for $ %}"; "reports_invalid_for_variable")]
#[test_case(b"{% for x %}"; "reports_missing_in_keyword")]
#[test_case(b"{% for x in %}"; "reports_missing_iterable")]
#[test_case(b"{% if true %}hi"; "reports_unclosed_if_block")]
#[test_case(b"{% for x in list %}hi"; "reports_unclosed_for_block")]
#[test_case(b"{% if true %}{% else x %}{% end %}"; "reports_else_with_operand")]
// render-time errors
#[test_case(b"{{ missing }}"; "reports_undefined_variable")]
#[test_case(b"{{ map.nope }}"; "reports_missing_map_key")]
#[test_case(b"{{ list.5 }}"; "reports_list_index_out_of_bounds")]
#[test_case(b"{{ list.-1 }}"; "reports_negative_list_index")]
#[test_case(b"{% if str %}x{% end %}"; "reports_if_condition_not_bool")]
#[test_case(b"{% for x in str %}x{% end %}"; "reports_for_iterable_not_list")]
#[test_case(b"{{ str.field }}"; "reports_dot_access_on_non_map")]
#[test_case(b"{{ str.0 }}"; "reports_index_access_on_non_list")]
// function errors
#[test_case(b"{{ nope() }}"; "reports_undefined_function")]
#[test_case(b"{{ one_arg() }}"; "reports_wrong_arg_count_zero")]
#[test_case(b"{{ one_arg(1, 2) }}"; "reports_two_args_when_one_expected")]
#[test_case(b"{{ two_arg(1) }}"; "reports_one_arg_when_two_expected")]
#[test_case(b"{{ mismatch(42) }}"; "reports_arg_type_mismatch")]
#[test_case(b"{{ custom(1, 2) }}"; "reports_custom_function_error")]
#[test_case(b"{{ custom_empty() }}"; "reports_custom_error_without_arg_indexes")]
// span / source-window edge cases
#[test_case(b"hello {{  }}"; "reports_empty_interp_after_text")]
#[test_case(b"line1\nline2\n{{  }}"; "reports_empty_interp_on_later_line")]
#[test_case(b"{{\n  @\n}}"; "reports_unexpected_token_in_multiline")]
fn invalid_template_reports_diagnostic(src: &[u8]) {
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
