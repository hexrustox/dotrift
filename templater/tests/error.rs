mod common;

use std::collections::HashMap;

use common::{MockRegistry, var_scope};
use templater::{Error, Template};

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
    let template = Template::from_bytes(b"hello".to_vec()).expect("parse failed");
    let e = template
        .render(&mut FailingWriter, &HashMap::new(), &MockRegistry)
        .unwrap_err();
    assert!(matches!(e, Error::Io(_)), "expected IO error, got: {e:?}");
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
    let template = Template::from_bytes(b"".to_vec()).expect("parse failed");
    let e = template
        .render(&mut FlushFailingWriter, &var_scope(), &MockRegistry)
        .unwrap_err();
    assert!(matches!(e, Error::Io(_)), "expected IO error, got: {e:?}");
}
