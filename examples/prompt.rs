use std::fs;

use tempfile::tempdir;

fn main() {
    let tmp = tempdir().unwrap();
    let source = tmp.path().join("source");
    let target = tmp.path().join("target");

    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&target).unwrap();
    let state = tmp.path().join("state");
    unsafe {
        std::env::set_var("DOTRIFT_PAGER", "less");
        std::env::set_var("XDG_STATE_HOME", &state);
    }

    fs::write(source.join("dotrift.toml"), "[portal]\n\"**\" = \".\"\n").unwrap();
    fs::write(source.join("file1"), "new").unwrap();
    fs::write(source.join("file2"), "new").unwrap();
    fs::write(target.join("file1"), "old").unwrap();
    fs::write(target.join("file2"), "old").unwrap();

    let status = dotrift::commands::apply::run(&source, Some(target.clone())).unwrap();

    for f in ["file1", "file2"] {
        println!("{f}: {}", fs::read_to_string(target.join(f)).unwrap());
    }
    println!("{status:?}");
}
