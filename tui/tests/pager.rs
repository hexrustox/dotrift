use std::{fs, io::Write};
use tempfile::{NamedTempFile, TempDir};
use tui::pager::PagerArgs;

#[ignore]
#[test]
fn view() {
    let mut file = NamedTempFile::new().unwrap();
    for i in 1..=20 {
        writeln!(file, "line {i}").unwrap();
    }
    file.flush().unwrap();

    tui::pager::run(PagerArgs::View(file.path())).unwrap();
}

#[ignore]
#[test]
fn diff() {
    let mut file1 = NamedTempFile::new().unwrap();
    let mut file2 = NamedTempFile::new().unwrap();
    for i in 1..=10 {
        writeln!(file1, "line {i}").unwrap();
        writeln!(file2, "line {}", 10 - i + 1).unwrap();
    }
    file1.flush().unwrap();
    file2.flush().unwrap();

    tui::pager::run(PagerArgs::Diff {
        source: file2.path(),
        target: file1.path(),
    })
    .unwrap();
}

#[ignore]
#[test]
fn explorer() {
    let mut file = NamedTempFile::new().unwrap();
    let dir = TempDir::new().unwrap();
    for i in 1..=100 {
        writeln!(file, "line {i}").unwrap();
    }
    file.flush().unwrap();
    for i in 1..=100 {
        fs::write(dir.path().join(format!("file{}", i)), "").unwrap();
    }
    tui::pager::run(PagerArgs::Explorer {
        source: file.path(),
        target: dir.path(),
    })
    .unwrap();
}
