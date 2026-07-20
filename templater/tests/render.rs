mod common;

use std::{collections::HashMap, io};

use common::{MockRegistry, var_scope};
use templater::Template;
use test_case::test_case;

/// Renders `source` with no variables.
pub fn render(source: &[u8]) -> Vec<u8> {
    let template = Template::from_bytes(source.to_vec()).expect("parse failed");
    let mut out = Vec::new();
    template
        .render(&mut out, &var_scope(), &MockRegistry)
        .expect("render failed");
    out
}

// --- Plain text ----------------------------------------------------------
#[test_case(b"hello, world\nthis is plain text" => b"hello, world\nthis is plain text".to_vec() ; "verbatim")]
#[test_case(b"bin\x80ary\xff\xfedata\x00here" => b"bin\x80ary\xff\xfedata\x00here".to_vec() ; "non_utf8_bytes")]
#[test_case(b"" => Vec::<u8>::new() ; "empty")]
// --- Interpolation: literals ---------------------------------------------
#[test_case(br#"{{ "literal" }}"# => b"literal".to_vec() ; "string_padded")]
#[test_case(br#"{{"literal"}}"# => b"literal".to_vec() ; "string_no_padding")]
#[test_case(br#"{{   "literal"   }}"# => b"literal".to_vec() ; "string_arbitrary_padding")]
#[test_case(br#"{{ "a\"b" }}"# => br#"a"b"#.to_vec() ; "string_escape_quote")]
#[test_case(br#"{{ "a\\b" }}"# => br#"a\b"#.to_vec() ; "string_escape_backslash")]
#[test_case(br#"{{ "a\xb" }}"# => br#"a\xb"#.to_vec() ; "string_other_escape_passes_through")]
#[test_case(b"{{ \"line1\nline2\" }}" => b"line1\nline2".to_vec() ; "string_preserves_raw_newline")]
#[test_case(br#"{{ "}}" }}"# => b"}}".to_vec() ; "string_shields_closing_delim")]
#[test_case(br#"{{ "\}}" }}"# => b"\\}}".to_vec() ; "string_backslash_quote_before_close_delim")]
#[test_case(b"{{ \"caf\xc3\xa9\" }}" => b"caf\xc3\xa9".to_vec() ; "string_preserves_non_ascii_bytes")]
#[test_case(b"{{ 42 }}" => b"42".to_vec() ; "int_positive")]
#[test_case(b"{{ -7 }}" => b"-7".to_vec() ; "int_negative")]
#[test_case(b"{{ 007 }}" => b"7".to_vec() ; "int_leading_zeros")]
#[test_case(b"{{ -0 }}" => b"0".to_vec() ; "int_negative_zero")]
#[test_case(b"{{ -9223372036854775808 }}" => b"-9223372036854775808".to_vec() ; "int_min_i64")]
#[test_case(b"{{ 9223372036854775807 }}" => b"9223372036854775807".to_vec() ; "int_max_i64")]
#[test_case(b"{{ true }}" => b"true".to_vec() ; "bool_true")]
#[test_case(b"{{ false }}" => b"false".to_vec() ; "bool_false")]
// --- Interpolation: variables --------------------------------------------
#[test_case(b"{{ name }}" => b"world".to_vec() ; "var_string")]
#[test_case(b"{{ count }}" => b"42".to_vec() ; "var_int")]
#[test_case(b"{{ neg }}" => b"-5".to_vec() ; "var_negative_int")]
#[test_case(b"{{ flag }}" => b"true".to_vec() ; "var_bool_true")]
#[test_case(b"{{ off }}" => b"false".to_vec() ; "var_bool_false")]
#[test_case(b"hello {{ name }}!" => b"hello world!".to_vec() ; "var_mixed_with_text")]
#[test_case(b"{{ count }} and {{ neg }} and {{ flag }}" => b"42 and -5 and true".to_vec() ; "var_multiple_in_sequence")]
#[test_case(b"prefix {{   name   }} suffix" => b"prefix world suffix".to_vec() ; "var_drops_padding")]
// --- Scanner: escape rules -----------------------------------------------
#[test_case(br"before \{{ after" => br"before {{ after".to_vec() ; "escape_escaped_open_renders_literal_braces")]
#[test_case(br"\{{\}}" => br"{{}}".to_vec() ; "escape_escaped_open_and_close")]
#[test_case(br"\\{{ name }}" => br"\world".to_vec() ; "escape_two_backslashes_keep_tag_active")]
#[test_case(br"\\\{{" => br"\{{".to_vec() ; "escape_three_backslashes_escape_tag")]
#[test_case(br"\\\\{{ name }}" => br"\\world".to_vec() ; "escape_four_backslashes_keep_tag_active")]
#[test_case(br"\{{ name \}}" => br"{{ name }}".to_vec() ; "escape_escaped_interp_pair_renders_literal")]
#[test_case(br"\{% name \%}" => br"{% name %}".to_vec() ; "escape_escaped_stmt_pair_renders_literal")]
#[test_case(br"\{# c \#}" => br"{# c #}".to_vec() ; "escape_escaped_comment_pair_renders_literal")]
#[test_case(br"\{%\%}" => br"{%%}".to_vec() ; "escape_escaped_empty_stmt_pair")]
#[test_case(br"\{#\#}" => br"{##}".to_vec() ; "escape_escaped_empty_comment_pair")]
#[test_case(br"\{{- name \-}}" => br"{{- name -}}".to_vec() ; "escape_escaped_tag_with_dash_modifiers")]
#[test_case(br"\{{= name \=}}" => br"{{= name =}}".to_vec() ; "escape_escaped_tag_with_equal_modifiers")]
#[test_case(br"\\" => br"\\".to_vec() ; "escape_even_backslashes_without_delimiter")]
// --- Scanner: comments ---------------------------------------------------
#[test_case(b"{# secret #}visible" => b"visible".to_vec() ; "comment_stripped")]
#[test_case(b"before {# c #} after" => b"before  after".to_vec() ; "comment_splits_plain_text")]
#[test_case(b"{#\n#}" => b"".to_vec() ; "comment_multiline_stripped")]
#[test_case(br"{# foo \#} bar #}" => b"".to_vec() ; "comment_escaped_close_is_stripped")]
#[test_case(b"visible{# c #}" => b"visible".to_vec() ; "comment_at_eof")]
#[test_case(b"{# c #}\nvisible" => b"\nvisible".to_vec() ; "comment_at_sof_preserves_newline")]
// --- Scanner: dash whitespace modifier -----------------------------------
#[test_case(b"  {{- name }}" => b"world".to_vec() ; "dash_left_trims_spaces")]
#[test_case(b"{{ name -}}  " => b"world".to_vec() ; "dash_right_trims_spaces")]
#[test_case(b"\t{{- name }}" => b"world".to_vec() ; "dash_left_trims_tab")]
#[test_case(b"{{ name -}}\t" => b"world".to_vec() ; "dash_right_trims_tab")]
#[test_case(b"before\n  {{- name }}" => b"before\nworld".to_vec() ; "dash_preserves_newline_left")]
#[test_case(b"before\n\t{{- name }}" => b"before\nworld".to_vec() ; "dash_preserves_newline_then_tab")]
// --- Scanner: equal whitespace modifier ----------------------------------
#[test_case(b"prefix {{= name }}" => b"world".to_vec() ; "equal_left_eats_to_line_start")]
#[test_case(b"{{ name =}}suffix\nnext" => b"worldnext".to_vec() ; "equal_right_eats_through_newline")]
#[test_case(b"{{ name =}} mid {{= name }}" => b"worldworld".to_vec() ; "equal_tags_share_line_delete_between")]
#[test_case(b"{{ name =}} {# c #} {{= name }}" => b"worldworld".to_vec() ; "equal_respects_comment_barrier")]
#[test_case(b"keep\nremove {{= name }}" => b"keep\nworld".to_vec() ; "equal_stops_before_newline_left")]
#[test_case(b"a\r\nkeep {{= name }}" => b"a\r\nworld".to_vec() ; "equal_eats_cr_as_plain_text")]
#[test_case(b"{{ name =}} keep {{ name }}" => b"worldworld".to_vec() ; "equal_right_stops_before_next_tag")]
#[test_case(b"{{ name }} keep {{= name }}" => b"worldworld".to_vec() ; "equal_left_stops_after_prev_close_tag")]
#[test_case(b"prefix {{= name =}}suffix\nnext" => b"worldnext".to_vec() ; "equal_left_and_right_combined_eat_suffix_line")]
#[test_case(b"{{ name =}}\t\nnext" => b"worldnext".to_vec() ; "equal_right_eats_tabs_and_newline")]
#[test_case(b"prefix\t{{= name }}" => b"world".to_vec() ; "equal_left_eats_tab_before_tag")]
#[test_case(br"text \\{{= name }}" => b"world".to_vec() ; "equal_left_eats_literal_backslashes")]
#[test_case(br"\\{{= name =}}" => b"world".to_vec() ; "equal_both_sides_eat_literal_backslashes")]
#[test_case(br"\{#= x \#}" => br"{#= x #}".to_vec() ; "equal_preserves_internal_modifier_chars")]
fn render_cases(source: &[u8]) -> Vec<u8> {
    render(source)
}

// --- Flush behavior -------------------------------------------------------

/// A writer that records its bytes and how often it was flushed.
#[derive(Default)]
struct FlushCounter {
    bytes: Vec<u8>,
    flushes: usize,
}

impl io::Write for FlushCounter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        Ok(())
    }
}

#[test]
fn render_flushes_writer_on_success() {
    let template = Template::from_bytes(b"abc".to_vec()).expect("parse failed");
    let mut writer = FlushCounter::default();
    template
        .render(&mut writer, &HashMap::new(), &MockRegistry)
        .expect("render failed");
    assert_eq!(writer.flushes, 1);
    assert_eq!(writer.bytes, b"abc");
}
