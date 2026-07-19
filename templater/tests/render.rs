mod common;

use std::{collections::HashMap, io};

use common::{MockRegistry, render};
use templater::{Template, Value};

#[test]
fn plain_text_renders_verbatim() {
    let source = b"hello, world\nthis is plain text";
    let out = render(source, &HashMap::new(), &MockRegistry);
    assert_eq!(out, source);
}

#[test]
fn non_utf8_bytes_render_verbatim() {
    let source = b"bin\x80ary\xff\xfedata\x00here";
    let out = render(source, &HashMap::new(), &MockRegistry);
    assert_eq!(out, source);
}

#[test]
fn empty_source_renders_empty() {
    let out = render(b"", &HashMap::new(), &MockRegistry);
    assert_eq!(out, b"");
}

// --- Interpolation: literals ---------------------------------------------

#[test]
fn interpolate_string_literal() {
    let out = render(br#"{{ "literal" }}"#, &HashMap::new(), &MockRegistry);
    assert_eq!(out, b"literal");
}

#[test]
fn interpolate_string_literal_no_padding() {
    let out = render(br#"{{"literal"}}"#, &HashMap::new(), &MockRegistry);
    assert_eq!(out, b"literal");
}

#[test]
fn interpolate_string_literal_arbitrary_padding() {
    let out = render(br#"{{   "literal"   }}"#, &HashMap::new(), &MockRegistry);
    assert_eq!(out, b"literal");
}

#[test]
fn interpolate_string_literal_escape_quote() {
    let out = render(br#"{{ "a\"b" }}"#, &HashMap::new(), &MockRegistry);
    assert_eq!(out, br#"a"b"#);
}

#[test]
fn interpolate_string_literal_escape_backslash() {
    let out = render(br#"{{ "a\\b" }}"#, &HashMap::new(), &MockRegistry);
    assert_eq!(out, br#"a\b"#);
}

#[test]
fn interpolate_string_literal_other_escape_passes_through_verbatim() {
    // `\X` for any X not `"` or `\` renders both bytes verbatim.
    let out = render(br#"{{ "a\xb" }}"#, &HashMap::new(), &MockRegistry);
    assert_eq!(out, br#"a\xb"#);
}

#[test]
fn interpolate_string_literal_preserves_raw_newline() {
    let out = render(b"{{ \"line1\nline2\" }}", &HashMap::new(), &MockRegistry);
    assert_eq!(out, b"line1\nline2");
}

#[test]
fn interpolate_string_literal_shields_closing_delim() {
    // `}}` inside a closed string literal does not close the tag.
    let out = render(br#"{{ "}}" }}"#, &HashMap::new(), &MockRegistry);
    assert_eq!(out, b"}}");
}

#[test]
fn interpolate_string_literal_preserves_non_ascii_bytes() {
    // String literals walk bytes directly into the writer — no char cast —
    // so non-ASCII byte sequences (here: the UTF-8 encoding of `é`) survive
    // intact even though they aren't valid standalone UTF-8 codepoints.
    let out = render(b"{{ \"caf\xc3\xa9\" }}", &HashMap::new(), &MockRegistry);
    assert_eq!(out, b"caf\xc3\xa9");
}

#[test]
fn interpolate_int_positive() {
    let out = render(b"{{ 42 }}", &HashMap::new(), &MockRegistry);
    assert_eq!(out, b"42");
}

#[test]
fn interpolate_int_negative() {
    let out = render(b"{{ -7 }}", &HashMap::new(), &MockRegistry);
    assert_eq!(out, b"-7");
}

#[test]
fn interpolate_int_leading_zeros() {
    let out = render(b"{{ 007 }}", &HashMap::new(), &MockRegistry);
    assert_eq!(out, b"7");
}

#[test]
fn interpolate_int_min_i64() {
    let out = render(
        b"{{ -9223372036854775808 }}",
        &HashMap::new(),
        &MockRegistry,
    );
    assert_eq!(out, b"-9223372036854775808");
}

#[test]
fn interpolate_int_max_i64() {
    let out = render(b"{{ 9223372036854775807 }}", &HashMap::new(), &MockRegistry);
    assert_eq!(out, b"9223372036854775807");
}

#[test]
fn interpolate_bool_true() {
    let out = render(b"{{ true }}", &HashMap::new(), &MockRegistry);
    assert_eq!(out, b"true");
}

#[test]
fn interpolate_bool_false() {
    let out = render(b"{{ false }}", &HashMap::new(), &MockRegistry);
    assert_eq!(out, b"false");
}

// --- Interpolation: variables -------------------------------------------

fn var_scope() -> HashMap<String, Value> {
    HashMap::from([
        ("name".to_string(), Value::Str("world".to_string())),
        ("count".to_string(), Value::Int(42)),
        ("neg".to_string(), Value::Int(-5)),
        ("flag".to_string(), Value::Bool(true)),
        ("off".to_string(), Value::Bool(false)),
    ])
}

#[test]
fn interpolate_string_var() {
    let out = render(b"{{ name }}", &var_scope(), &MockRegistry);
    assert_eq!(out, b"world");
}

#[test]
fn interpolate_int_var() {
    let out = render(b"{{ count }}", &var_scope(), &MockRegistry);
    assert_eq!(out, b"42");
}

#[test]
fn interpolate_negative_int_var() {
    let out = render(b"{{ neg }}", &var_scope(), &MockRegistry);
    assert_eq!(out, b"-5");
}

#[test]
fn interpolate_bool_var_true() {
    let out = render(b"{{ flag }}", &var_scope(), &MockRegistry);
    assert_eq!(out, b"true");
}

#[test]
fn interpolate_bool_var_false() {
    let out = render(b"{{ off }}", &var_scope(), &MockRegistry);
    assert_eq!(out, b"false");
}

#[test]
fn interpolate_mixed_with_text() {
    let out = render(b"hello {{ name }}!", &var_scope(), &MockRegistry);
    assert_eq!(out, b"hello world!");
}

#[test]
fn interpolate_multiple_in_sequence() {
    let out = render(
        b"{{ count }} and {{ neg }} and {{ flag }}",
        &var_scope(),
        &MockRegistry,
    );
    assert_eq!(out, b"42 and -5 and true");
}

#[test]
fn interpolate_var_drops_surrounding_padding() {
    let out = render(b"prefix {{   name   }} suffix", &var_scope(), &MockRegistry);
    assert_eq!(out, b"prefix world suffix");
}

// ---Flush behavior -------------------------------------------------------

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
