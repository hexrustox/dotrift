use std::{fs, io::Write};
use tempfile::{NamedTempFile, TempDir};

#[ignore]
#[test]
fn file() {
    let mut file = NamedTempFile::new().unwrap();
    for i in 1..=100 {
        writeln!(file, "line {i}").unwrap();
    }
    file.flush().unwrap();

    tui::pager::run(file.path(), None).unwrap();
}

#[ignore]
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

    tui::pager::run(file1.path(), Some(file2.path())).unwrap();
}

#[ignore]
#[test]
fn explorer() {
    let mut path1 = NamedTempFile::new().unwrap();
    let path2 = TempDir::new().unwrap();
    for i in 1..=100 {
        writeln!(path1, "line {i}").unwrap();
    }
    path1.flush().unwrap();
    for i in 1..=100 {
        fs::write(path2.path().join(format!("file{}", i)), "").unwrap();
    }
    tui::pager::run(path1.path(), Some(path2.path())).unwrap();
}
