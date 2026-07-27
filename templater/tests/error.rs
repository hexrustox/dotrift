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

fn assert_root_io_error(report: &miette::Report) {
    let err = report.downcast_ref::<Error>().expect("root error is Error");
    assert!(
        matches!(err, Error::Io(_)),
        "expected IO error, got: {err:?}"
    );
}

#[test]
fn render_propagates_io_error() {
    let template = Template::from_bytes(b"hello".to_vec()).expect("parse failed");
    let report = template
        .render(&mut FailingWriter, &HashMap::new(), &MockRegistry)
        .unwrap_err();
    assert_root_io_error(&report);
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
    let report = template
        .render(&mut FlushFailingWriter, &var_scope(), &MockRegistry)
        .unwrap_err();
    assert_root_io_error(&report);
}

#[test]
fn from_file_missing_file_is_io_error() {
    let err = Template::from_file("/definitely/does/not/exist.dotrift").unwrap_err();
    assert!(
        matches!(err, Error::Io(_)),
        "expected IO error, got: {err:?}"
    );
}

#[test]
fn from_file_directory_is_io_error() {
    let temp = tempfile::tempdir().expect("temp dir");
    let err = Template::from_file(temp.path()).unwrap_err();
    assert!(
        matches!(err, Error::Io(_)),
        "expected IO error, got: {err:?}"
    );
}
