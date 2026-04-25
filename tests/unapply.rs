use dotrift::{
    cli::{ApplyFlags, UnapplyFlags},
    command::{apply, unapply},
};

use crate::common::{flags, setup_test};

mod common;

#[test]
fn test_unapply() {
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
    unapply::run(
        flags(&source_dir, &target_dir),
        &db_path,
        UnapplyFlags {
            dry_run: false,
            prune_empty_dirs: true,
        },
    )
    .unwrap();

    assert!(!target_dir.join("a.txt").exists());
    assert!(!target_dir.join("b.txt").exists());
    assert!(!target_dir.join("subdir").exists());
    assert!(!target_dir.join("subdir/c.txt").exists());
    assert!(!target_dir.join("subdir/d.txt").exists());
}

#[test]
#[ignore]
fn test_unapply_dry_run() {
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
    unapply::run(
        flags(&source_dir, &target_dir),
        &db_path,
        UnapplyFlags {
            dry_run: true,
            prune_empty_dirs: false,
        },
    )
    .unwrap();
}
