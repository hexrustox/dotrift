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

#[test]
fn plain_text_renders_verbatim() {
    let source = b"hello, world\nthis is plain text";
    assert_eq!(render(source), source);
}

#[test]
fn non_utf8_bytes_render_verbatim() {
    let source = b"bin\x80ary\xff\xfedata\x00here";
    assert_eq!(render(source), source);
}

#[test]
fn empty_source_renders_empty() {
    assert_eq!(render(b""), b"");
}

// --- Interpolation: literals ---------------------------------------------

#[test_case(br#"{{ "literal" }}"# => b"literal".to_vec() ; "padded")]
#[test_case(br#"{{"literal"}}"# => b"literal".to_vec() ; "no_padding")]
#[test_case(br#"{{   "literal"   }}"# => b"literal".to_vec() ; "arbitrary_padding")]
#[test_case(br#"{{ "a\"b" }}"# => br#"a"b"#.to_vec() ; "escape_quote")]
#[test_case(br#"{{ "a\\b" }}"# => br#"a\b"#.to_vec() ; "escape_backslash")]
// `\X` for any X not `"` or `\` renders both bytes verbatim.
#[test_case(br#"{{ "a\xb" }}"# => br#"a\xb"#.to_vec() ; "other_escape_passes_through")]
// Raw newlines inside string literals are preserved.
#[test_case(b"{{ \"line1\nline2\" }}" => b"line1\nline2".to_vec() ; "preserves_raw_newline")]
// `}}` inside a closed string literal does not close the tag.
#[test_case(br#"{{ "}}" }}"# => b"}}".to_vec() ; "shields_closing_delim")]
// String literals walk bytes directly into the writer — no char cast —
// so non-ASCII byte sequences (here: the UTF-8 encoding of `é`) survive
// intact even though they aren't valid standalone UTF-8 codepoints.
#[test_case(b"{{ \"caf\xc3\xa9\" }}" => b"caf\xc3\xa9".to_vec() ; "preserves_non_ascii_bytes")]
fn interpolate_string_literal(source: &[u8]) -> Vec<u8> {
    render(source)
}

#[test_case(b"{{ 42 }}" => b"42".to_vec() ; "positive")]
#[test_case(b"{{ -7 }}" => b"-7".to_vec() ; "negative")]
#[test_case(b"{{ 007 }}" => b"7".to_vec() ; "leading_zeros")]
#[test_case(b"{{ -0 }}" => b"0".to_vec() ; "negative_zero")]
#[test_case(b"{{ -9223372036854775808 }}" => b"-9223372036854775808".to_vec() ; "min_i64")]
#[test_case(b"{{ 9223372036854775807 }}" => b"9223372036854775807".to_vec() ; "max_i64")]
fn interpolate_int_literal(source: &[u8]) -> Vec<u8> {
    render(source)
}

#[test_case(b"{{ true }}" => b"true".to_vec() ; "bool_true")]
#[test_case(b"{{ false }}" => b"false".to_vec() ; "bool_false")]
fn interpolate_bool_literal(source: &[u8]) -> Vec<u8> {
    render(source)
}

// --- Interpolation: variables -------------------------------------------

#[test_case(b"{{ name }}" => b"world".to_vec() ; "string")]
#[test_case(b"{{ count }}" => b"42".to_vec() ; "int")]
#[test_case(b"{{ neg }}" => b"-5".to_vec() ; "negative_int")]
#[test_case(b"{{ flag }}" => b"true".to_vec() ; "bool_true")]
#[test_case(b"{{ off }}" => b"false".to_vec() ; "bool_false")]
#[test_case(b"hello {{ name }}!" => b"hello world!".to_vec() ; "mixed_with_text")]
#[test_case(b"{{ count }} and {{ neg }} and {{ flag }}" => b"42 and -5 and true".to_vec() ; "multiple_in_sequence")]
#[test_case(b"prefix {{   name   }} suffix" => b"prefix world suffix".to_vec() ; "drops_padding")]
fn interpolate_var(source: &[u8]) -> Vec<u8> {
    render(source)
}

// --- Scanner: escape rules ------------------------------------------------

#[test_case(b"before \\{{ after" => b"before {{ after".to_vec() ; "escaped_open_renders_literal_braces")]
#[test_case(b"\\{{\\}}" => b"{{}}".to_vec() ; "escaped_open_and_close")]
#[test_case(b"\\\\{{ name }}" => b"\\world".to_vec() ; "two_backslashes_keep_tag_active")]
#[test_case(b"\\\\\\{{" => b"\\{{".to_vec() ; "three_backslashes_escape_tag")]
#[test_case(b"\\\\\\\\{{ name }}" => b"\\\\world".to_vec() ; "four_backslashes_keep_tag_active")]
#[test_case(b"\\{{ name \\}}" => b"{{ name }}".to_vec() ; "escaped_interp_pair_renders_literal")]
#[test_case(b"\\{% name \\%}" => b"{% name %}".to_vec() ; "escaped_stmt_pair_renders_literal")]
#[test_case(b"\\{# c \\#}" => b"{# c #}".to_vec() ; "escaped_comment_pair_renders_literal")]
#[test_case(b"\\{%\\%}" => b"{%%}".to_vec() ; "escaped_empty_stmt_pair")]
#[test_case(b"\\{#\\#}" => b"{##}".to_vec() ; "escaped_empty_comment_pair")]
#[test_case(b"\\{{- name \\-}}" => b"{{- name -}}".to_vec() ; "escaped_tag_with_dash_modifiers")]
#[test_case(b"\\{{= name \\=}}" => b"{{= name =}}".to_vec() ; "escaped_tag_with_equal_modifiers")]
#[test_case(b"\\\\" => b"\\\\".to_vec() ; "even_backslashes_without_delimiter")]
fn escaped_delimiters_render_literal(source: &[u8]) -> Vec<u8> {
    render(source)
}

// --- Scanner: comments ----------------------------------------------------

#[test_case(b"{# secret #}visible" => b"visible".to_vec() ; "stripped")]
#[test_case(b"before {# c #} after" => b"before  after".to_vec() ; "splits_plain_text")]
#[test_case(b"{#\n#}" => b"".to_vec() ; "multiline_stripped")]
// Escaped `#}` inside a comment is treated as literal text and stripped along
// with the comment; the comment continues until the next unescaped `#}`.
#[test_case(b"{# foo \\#} bar #}" => b"".to_vec() ; "escaped_close_is_stripped")]
#[test_case(b"visible{# c #}" => b"visible".to_vec() ; "at_eof")]
#[test_case(b"{# c #}\nvisible" => b"\nvisible".to_vec() ; "at_sof_preserves_newline")]
fn comment_is_stripped(source: &[u8]) -> Vec<u8> {
    render(source)
}

// --- Scanner: dash whitespace modifier ------------------------------------

#[test_case(b"  {{- name }}" => b"world".to_vec() ; "left_trims_spaces")]
#[test_case(b"{{ name -}}  " => b"world".to_vec() ; "right_trims_spaces")]
#[test_case(b"\t{{- name }}" => b"world".to_vec() ; "left_trims_tab")]
#[test_case(b"{{ name -}}\t" => b"world".to_vec() ; "right_trims_tab")]
#[test_case(b"before\n  {{- name }}" => b"before\nworld".to_vec() ; "preserves_newline_left")]
#[test_case(b"before\n\t{{- name }}" => b"before\nworld".to_vec() ; "preserves_newline_then_tab")]
fn dash_trims_adjacent_spaces_and_tabs(source: &[u8]) -> Vec<u8> {
    render(source)
}

// --- Scanner: equal whitespace modifier -----------------------------------

#[test_case(b"prefix {{= name }}" => b"world".to_vec() ; "left_eats_to_line_start")]
#[test_case(b"{{ name =}}suffix\nnext" => b"worldnext".to_vec() ; "right_eats_through_newline")]
#[test_case(b"{{ name =}} mid {{= name }}" => b"worldworld".to_vec() ; "tags_share_line_delete_between")]
#[test_case(b"{{ name =}} {# c #} {{= name }}" => b"worldworld".to_vec() ; "respects_comment_barrier")]
#[test_case(b"keep\nremove {{= name }}" => b"keep\nworld".to_vec() ; "stops_before_newline_left")]
// `\r` is not a line terminator, so left `=` stops at the real `\n` and the
// `\r` and the `\n` both survive.
#[test_case(b"a\r\nkeep {{= name }}" => b"a\r\nworld".to_vec() ; "eats_cr_as_plain_text")]
#[test_case(b"{{ name =}} keep {{ name }}" => b"worldworld".to_vec() ; "right_stops_before_next_tag")]
#[test_case(b"{{ name }} keep {{= name }}" => b"worldworld".to_vec() ; "left_stops_after_prev_close_tag")]
#[test_case(b"prefix {{= name =}}suffix\nnext" => b"worldnext".to_vec() ; "left_and_right_combined_eat_suffix_line")]
#[test_case(b"{{ name =}}\t\nnext" => b"worldnext".to_vec() ; "right_eats_tabs_and_newline")]
#[test_case(b"prefix\t{{= name }}" => b"world".to_vec() ; "left_eats_tab_before_tag")]
fn equal_modifier_eats_plain_text_and_whitespace(source: &[u8]) -> Vec<u8> {
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
