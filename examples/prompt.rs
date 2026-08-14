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

    fs::write(
        source.join("dotrift.toml"),
        "[portal]\n\"greeting.txt\" = \"greeting.txt\"\n",
    )
    .expect("cannot write dotrift.toml");
    fs::write(source.join("greeting.txt"), "hello from the dotrift source")
        .expect("cannot write source file");
    fs::write(target.join("greeting.txt"), "this file was already here")
        .expect("cannot write obstructing target file");

    let status = dotrift::commands::apply::run(&source, Some(target.clone())).unwrap();

    println!(
        "{}",
        fs::read_to_string(target.join("greeting.txt")).unwrap()
    );
    println!("{status:?}");
}
