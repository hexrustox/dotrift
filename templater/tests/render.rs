mod common;

use std::{collections::HashMap, io};

use common::{MockRegistry, var_scope};
use templater::{Template, util::TestRegistry};
use test_case::test_case;

#[test_case(b"" => ""; "empty")]
#[test_case(b"hello" => "hello"; "plain_text")]
#[test_case(b"{# comment #}" => ""; "comment_only")]
#[test_case(b"a{#c#}b" => "ab"; "comment_between_text")]
#[test_case(b"{{ str }}" => "foobar"; "var_str")]
#[test_case(b"{{ num }}" => "42"; "var_int")]
#[test_case(b"{{ neg }}" => "-5"; "var_neg_int")]
#[test_case(b"{{ yes }}" => "true"; "var_bool_true")]
#[test_case(b"{{ no }}" => "false"; "var_bool_false")]
#[test_case(b"{{ empty_list }}" => "[]"; "var_empty_list")]
#[test_case(b"{{ list }}" => "[1, 2, 3]"; "var_list")]
#[test_case(b"{{ map }}" => r#"{"key": "value", "nested": {"nested": "value"}}"#; "var_map")]
#[test_case(br#"{{ ["\"", "\\"] }}"# => r#"["\"", "\\"]"#; "list_literal_with_escapes")]
#[test_case(br#"{{"string"}}"# => "string"; "string_literal")]
#[test_case(br#"{{"with \" quote"}}"# => "with \" quote"; "string_literal_escaped_quote")]
#[test_case(br#"{{"with \\ backslash"}}"# => "with \\ backslash"; "string_literal_escaped_backslash")]
#[test_case(br#"{{"unknown \\x escape"}}"# => "unknown \\x escape"; "string_literal_unknown_escape")]
#[test_case(b"{{\"line1\nline2\"}}" => "line1\nline2"; "string_literal_multiline")]
#[test_case(b"{{ 42 }}" => "42"; "int_literal_positive")]
#[test_case(b"{{ -7 }}" => "-7"; "int_literal_negative")]
#[test_case(b"{{ 007 }}" => "7"; "int_literal_leading_zeros")]
#[test_case(b"{{ true }}" => "true"; "bool_literal_true")]
#[test_case(b"{{ false }}" => "false"; "bool_literal_false")]
#[test_case(b"{{ [] }}" => "[]"; "list_literal_empty")]
#[test_case(b"{{ [\"a\", \"b\"] }}" => r#"["a", "b"]"#; "list_literal_strings")]
#[test_case(b"{{ [1, true, [\"nested\"]] }}" => "[1, true, [\"nested\"]]"; "list_literal_nested")]
#[test_case(b"{{ map.key }}" => "value"; "map_field_access")]
#[test_case(b"{{ map.nested.nested }}" => "value"; "nested_map_field_access")]
#[test_case(b"{{ list.0 }}" => "1"; "list_index_access")]
#[test_case(b"{{ same(map).key }}" => "value"; "dot_after_fn_call")]
#[test_case(b"{{ foo() }}" => "bar"; "zero_arg_fn_call")]
#[test_case(b"{{ same(\"echo\") }}" => "echo"; "one_arg_fn_call")]
#[test_case(b"  {{- str -}}  " => "foobar"; "interp_trim_left_right")]
#[test_case(b"{{ str =}} mid {{= str }}" => "foobarfoobar"; "interp_eq_both_sides")]
#[test_case(b"{% if yes %}Y{% end %}" => "Y"; "if_true")]
#[test_case(b"{% if no %}Y{% else %}N{% end %}" => "N"; "if_false_else")]
#[test_case(b"{% if no %}A{% elif yes %}B{% else %}C{% end %}" => "B"; "if_elif_true")]
#[test_case(b"{% if no %}A{% elif no %}B{% else %}C{% end %}" => "C"; "if_else_fallback")]
#[test_case(b"{% if yes %}{% if yes %}both{% end %}{% end %}" => "both"; "nested_if_true")]
#[test_case(b"{% if no %}A{% elif yes %}{% if no %}B{% else %}C{% end %}{% end %}" => "C"; "elif_body_with_nested_if")]
#[test_case(b"{% if false %}{{ missing }}{% end %}" => ""; "untaken_branch_silent")]
#[test_case(b"{% for x in list %}{{x}},{% end %}" => "1,2,3,"; "for_over_list")]
#[test_case(b"{% for x in empty_list %}{{x}}{% end %}" => ""; "for_over_empty_list")]
#[test_case(b"{% for x in [10, 20] %}{{x}};{% end %}" => "10;20;"; "for_over_list_literal")]
#[test_case(b"{% for x in same([1, 2, 3]) %}{{x}}{% end %}" => "123"; "for_over_fn_call")]
#[test_case(b"{% for x in list %}{% if yes %}{{x}}{% end %}{% end %}" => "123"; "nested_for_if")]
#[test_case(b"{% for str in list %}{{str}}{% end %}{{str}}" => "123foobar"; "for_shadows_outer_var")]
#[test_case(b"{% for x in [[1, 2], [3, 4]] %}{% for y in x %}{{y}}{% end %},{% end %}" => "12,34,"; "nested_for_shadowed_names")]
#[test_case(b"{% if yes %}{% for x in list %}{{x}}{% end %}{% end %}" => "123"; "if_wrapping_for")]
#[test_case(b"a{% if yes %}b{% end %}c" => "abc"; "text_around_if")]
fn render(bytes: &[u8]) -> String {
    let mut out = Vec::new();
    Template::from_bytes(bytes.to_vec())
        .unwrap()
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
    let template = Template::from_bytes(b"hello".to_vec()).expect("parse failed");
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
    Template::from_bytes(bytes.to_vec())
        .unwrap()
        .render(&mut bytes_out, &var_scope(), &TestRegistry)
        .expect("render failed");

    assert_eq!(file_out, bytes_out);
}
