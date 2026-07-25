mod common;

use std::{collections::HashMap, io};

use common::MockRegistry;
use templater::Template;

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
