use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn single_pane() {
    let mut file = NamedTempFile::new().unwrap();
    for i in 1..=100 {
        writeln!(file, "line {i}").unwrap();
    }
    file.flush().unwrap();

    let result = tui::pager::run(file.path(), None);
    assert!(result.is_ok());
}

#[test]
fn diff() {
    let mut file1 = NamedTempFile::new().unwrap();
    let mut file2 = NamedTempFile::new().unwrap();
    for i in 1..=50 {
        writeln!(file1, "line {i}").unwrap();
        writeln!(file2, "line {}", 50 - i + 1).unwrap();
    }
    file1.flush().unwrap();
    file2.flush().unwrap();

    let result = tui::pager::run(file1.path(), Some(file2.path()));
    assert!(result.is_ok());
}
