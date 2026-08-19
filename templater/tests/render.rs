mod common;

use std::{collections::HashMap, io};

use templater::{Template, util::TestRegistry};
use test_case::test_case;

use common::{MockRegistry, var_scope};

#[test_case(b"" => ""; "empty_template_renders_nothing")]
#[test_case(b"hello" => "hello"; "renders_plain_text_verbatim")]
#[test_case(b"{# comment #}" => ""; "comment_stripped_from_output")]
#[test_case(b"a{#c#}b" => "ab"; "strips_comment_between_text")]
#[test_case(b"{{ str }}" => "foobar"; "interpolates_string_variable")]
#[test_case(b"{{ num }}" => "42"; "renders_int_variable_as_decimal")]
#[test_case(b"{{ neg }}" => "-5"; "renders_negative_int_variable")]
#[test_case(b"{{ yes }}" => "true"; "renders_true_bool_variable")]
#[test_case(b"{{ no }}" => "false"; "renders_false_bool_variable")]
#[test_case(b"{{ empty_list }}" => "[]"; "renders_empty_list_variable")]
#[test_case(b"{{ list }}" => "[1, 2, 3]"; "renders_list_variable")]
#[test_case(b"{{ map }}" => r#"{"key": "value", "nested": {"nested": "value"}}"#; "renders_map_variable")]
#[test_case(br#"{{ ["\"", "\\"] }}"# => r#"["\"", "\\"]"#; "list_literal_preserves_escaped_bytes")]
#[test_case(br#"{{"string"}}"# => "string"; "string_literal_renders_payload")]
#[test_case(br#"{{"with \" quote"}}"# => "with \" quote"; "string_literal_decodes_escaped_quote")]
#[test_case(br#"{{"with \\ backslash"}}"# => "with \\ backslash"; "string_literal_decodes_escaped_backslash")]
#[test_case(br#"{{"unknown \\x escape"}}"# => "unknown \\x escape"; "string_literal_keeps_unknown_escape")]
#[test_case(b"{{\"line1\nline2\"}}" => "line1\nline2"; "string_literal_preserves_newline")]
#[test_case(b"{{ 42 }}" => "42"; "renders_positive_int_literal")]
#[test_case(b"{{ -7 }}" => "-7"; "renders_negative_int_literal")]
#[test_case(b"{{ 007 }}" => "7"; "normalizes_leading_zeros_in_int_literal")]
#[test_case(b"{{ true }}" => "true"; "renders_true_bool_literal")]
#[test_case(b"{{ false }}" => "false"; "renders_false_bool_literal")]
#[test_case(b"{{ [] }}" => "[]"; "renders_empty_list_literal")]
#[test_case(b"{{ [\"a\", \"b\"] }}" => r#"["a", "b"]"#; "renders_list_literal_of_strings")]
#[test_case(b"{{ [1, true, [\"nested\"]] }}" => "[1, true, [\"nested\"]]"; "renders_nested_list_literal")]
#[test_case(b"{{ map.key }}" => "value"; "dot_access_on_map_returns_value")]
#[test_case(b"{{ map.nested.nested }}" => "value"; "chained_dot_access_returns_nested_value")]
#[test_case(b"{{ list.0 }}" => "1"; "dot_index_on_list_returns_item")]
#[test_case(b"{{ same(map).key }}" => "value"; "dot_access_follows_fn_call_result")]
#[test_case(b"{{ foo() }}" => "bar"; "zero_arg_fn_call_returns_value")]
#[test_case(b"{{ same(\"echo\") }}" => "echo"; "one_arg_fn_call_returns_value")]
#[test_case(b"  {{- str -}}  " => "foobar"; "dash_trim_strips_surrounding_whitespace")]
#[test_case(b"{{ str =}} mid {{= str }}" => "foobarfoobar"; "equals_trim_strips_whitespace_on_both_sides")]
#[test_case(b"{% if yes %}Y{% end %}" => "Y"; "if_true_renders_body")]
#[test_case(b"{% if no %}Y{% else %}N{% end %}" => "N"; "if_false_renders_else_body")]
#[test_case(b"{% if no %}A{% elif yes %}B{% else %}C{% end %}" => "B"; "elif_true_renders_elif_body")]
#[test_case(b"{% if no %}A{% elif no %}B{% else %}C{% end %}" => "C"; "all_conditions_false_falls_back_to_else")]
#[test_case(b"{% if yes %}{% if yes %}both{% end %}{% end %}" => "both"; "nested_true_if_renders_inner_body")]
#[test_case(b"{% if no %}A{% elif yes %}{% if no %}B{% else %}C{% end %}{% end %}" => "C"; "nested_if_renders_inside_elif_body")]
#[test_case(b"{% if false %}{{ missing }}{% end %}" => ""; "untaken_branch_skips_undefined_variable")]
#[test_case(b"{% for x in list %}{{x}},{% end %}" => "1,2,3,"; "for_loop_renders_body_per_item")]
#[test_case(b"{% for x in empty_list %}{{x}}{% end %}" => ""; "for_loop_over_empty_list_renders_nothing")]
#[test_case(b"{% for x in [10, 20] %}{{x}};{% end %}" => "10;20;"; "for_loop_renders_body_per_literal_item")]
#[test_case(b"{% for x in same([1, 2, 3]) %}{{x}}{% end %}" => "123"; "for_loop_iterates_fn_call_result")]
#[test_case(b"{% for x in list %}{% if yes %}{{x}}{% end %}{% end %}" => "123"; "for_loop_body_can_contain_if")]
#[test_case(b"{% for str in list %}{{str}}{% end %}{{str}}" => "123foobar"; "for_variable_shadows_outer_scope")]
#[test_case(b"{% for x in [[1, 2], [3, 4]] %}{% for y in x %}{{y}}{% end %},{% end %}" => "12,34,"; "nested_for_loops_keep_distinct_variables")]
#[test_case(b"{% if yes %}{% for x in list %}{{x}}{% end %}{% end %}" => "123"; "if_body_can_contain_for_loop")]
#[test_case(b"a{% if yes %}b{% end %}c" => "abc"; "if_block_renders_text_around_it")]
fn renders_template_to_output(src: &[u8]) -> String {
    let mut out = Vec::new();
    Template::from_bytes(src)
        .render(&mut out, &var_scope(), &TestRegistry)
        .unwrap();
    String::from_utf8(out).unwrap()
}

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
    let template = Template::from_bytes(b"hello");
    let mut writer = FlushCounter::default();
    template
        .render(&mut writer, &HashMap::new(), &MockRegistry)
        .expect("render failed");
    assert_eq!(writer.flushes, 1);
    assert_eq!(writer.bytes, b"hello");
}

#[test]
fn from_file_renders_same_as_from_bytes() {
    let bytes = b"{% for x in list %}{{x}}{% end %}";
    let temp = tempfile::tempdir().expect("temp dir");
    let path = temp.path().join("template.txt");
    std::fs::write(&path, bytes).expect("write template");

    let mut file_out = Vec::new();
    Template::from_file(&path)
        .expect("from_file failed")
        .render(&mut file_out, &var_scope(), &TestRegistry)
        .expect("render failed");

    let mut bytes_out = Vec::new();
    Template::from_bytes(bytes)
        .render(&mut bytes_out, &var_scope(), &TestRegistry)
        .expect("render failed");

    assert_eq!(file_out, bytes_out);
}
