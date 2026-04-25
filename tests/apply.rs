use dotrift::cli::GlobalFlags;
use dotrift::{cli::ApplyFlags, command::apply};
use std::fs;

use crate::common::{flags, setup_test};

mod common;

#[test]
fn test_run() {
    let config = r#"
[portal]
"" = ""

[rule]
"subdir/*" = { type = "copy" }
"#;
    let (_temp_dir, source_dir, target_dir, db_path) = setup_test(config);

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

    assert!(!target_dir.join("dotrift.toml").exists());

    assert!(target_dir.join("a.txt").exists());
    assert!(target_dir.join("b.txt").exists());
    assert!(target_dir.join("a.txt").is_symlink());
    assert!(target_dir.join("b.txt").is_symlink());

    assert!(target_dir.join("subdir/c.txt").exists());
    assert!(target_dir.join("subdir/d.txt").exists());
    assert!(!target_dir.join("subdir/c.txt").is_symlink());
    assert!(!target_dir.join("subdir/d.txt").is_symlink());
}

#[test]
#[ignore]
fn test_dry_run() {
    let config = r#"
[portal]
"" = ""

[rule]
"subdir/*" = { type = "copy" }
"#;
    let (_temp_dir, source_dir, target_dir, db_path) = setup_test(config);

    apply::run(
        flags(&source_dir, &target_dir),
        &db_path,
        ApplyFlags {
            dry_run: true,
            clean_up: false,
            prune_empty_dirs: false,
        },
    )
    .unwrap();
}

#[test]
#[ignore]
fn test_clean_up_dry_run() {
    let config = r#"
[portal]
"" = ""
"#;
    let (_temp_dir, source_dir, target_dir, db_path) = setup_test(config);

    apply::run(
        flags(&source_dir, &target_dir),
        &db_path,
        ApplyFlags {
            dry_run: false,
            clean_up: false,
            prune_empty_dirs: false,
        },
    )
    .unwrap();

    let config = r#"
[portal]
"*.txt" = ""
"#;
    fs::write(source_dir.join("dotrift.toml"), config).unwrap();

    apply::run(
        flags(&source_dir, &target_dir),
        &db_path,
        ApplyFlags {
            dry_run: true,
            clean_up: true,
            prune_empty_dirs: false,
        },
    )
    .unwrap();
}
