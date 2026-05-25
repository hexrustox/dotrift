use std::fs;

use dotrift::{
    cli::{ApplyFlags, GlobalFlags},
    command::{apply, diff},
};

use crate::common::setup_test;

mod common;

#[ignore]
#[test]
fn test_diff() {
    let config = r#"
[portal]
"a.txt" = "a.txt"

[rule]
"**/*" = { type = "copy" }
"#;
    let (_temp_dir, source_dir, target_dir, db_path) = setup_test(config);

    fs::write(source_dir.join("a.txt"), "source").unwrap();
    apply::run(
        GlobalFlags::new(
            Some(source_dir.to_path_buf()),
            Some(target_dir.to_path_buf()),
            None,
        ),
        &db_path,
        ApplyFlags {
            dry_run: false,
            clean_up: false,
            prune_empty_dirs: false,
        },
    )
    .unwrap();

    fs::write(target_dir.join("a.txt"), "target").unwrap();
    diff::run(target_dir.join("a.txt"), &db_path).unwrap();
}
