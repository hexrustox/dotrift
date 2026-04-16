use dotrift::command::apply;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

fn setup_test() -> (TempDir, PathBuf, PathBuf, PathBuf) {
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    let target_dir = temp_dir.path().join("target");
    let db_path = temp_dir.path().join("db");

    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&target_dir).unwrap();
    fs::write(source_dir.join("a.txt"), "").unwrap();
    fs::write(source_dir.join("b.txt"), "").unwrap();
    fs::create_dir(source_dir.join("subdir")).unwrap();
    fs::write(source_dir.join("subdir").join("c.txt"), "").unwrap();
    fs::write(source_dir.join("subdir").join("d.txt"), "").unwrap();

    (temp_dir, source_dir, target_dir, db_path)
}

#[test]
fn test_dry_run() {
    let (_temp_dir, source_dir, target_dir, db_path) = setup_test();

    let config = r#"
[portal]
"" = ""

[rule]
"subdir/*" = { type = "copy" }
"#;
    fs::write(source_dir.join("dotrift.toml"), config).unwrap();

    let flags = dotrift::cli::ApplyFlags {
        dry_run: true,
        clean_up: false,
        prune_empty_dirs: false,
    };

    apply::run(
        source_dir.clone(),
        Some(target_dir.clone()),
        &db_path,
        flags,
    )
    .unwrap();
}
