use std::io::Write;
use tempfile::NamedTempFile;

#[test]
fn single_pane_basic() {
    let mut file = NamedTempFile::new().unwrap();
    for i in 1..=100 {
        writeln!(file, "line {i}").unwrap();
    }
    file.flush().unwrap();

    let result = tui::pager::run(file.path(), None);
    assert!(result.is_ok());
}
