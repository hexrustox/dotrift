mod common;

use std::{collections::HashMap, io};

use common::{MockRegistry, render};
use templater::Template;

#[test]
fn plain_text_renders_verbatim() {
    let source = b"hello, world\nthis is plain text";
    let out = render(source, &HashMap::new(), &MockRegistry);
    assert_eq!(out, source);
}

#[test]
fn tag_shaped_bytes_render_verbatim() {
    // The scanner is a passthrough in this slice: no tags are recognized, so
    // delimiter-shaped bytes are plain text.
    let source = b"a {{ x }} b {% if c %} d {# e #} f {{- g =}} h";
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
