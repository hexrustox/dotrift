use std::fs;

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
fn test_unapply_dry_run_no_files_removed() {
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

    assert!(target_dir.join("a.txt").exists());
    assert!(target_dir.join("subdir/c.txt").exists());

    unapply::run(
        flags(&source_dir, &target_dir),
        &db_path,
        UnapplyFlags {
            dry_run: true,
            prune_empty_dirs: false,
        },
    )
    .unwrap();

    assert!(target_dir.join("a.txt").exists());
    assert!(target_dir.join("subdir/c.txt").exists());

    let db = dotrift::db::Db::init(&db_path).unwrap();
    assert!(db.get_entry(&target_dir.join("a.txt")).unwrap().is_some());
    assert!(
        db.get_entry(&target_dir.join("subdir/c.txt"))
            .unwrap()
            .is_some()
    );
}

#[test]
fn test_unapply_copy_deploy() {
    let config = r#"
[portal]
"" = ""

[rule]
"**/*" = { type = "copy" }
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

    assert!(target_dir.join("a.txt").is_file());
    assert!(target_dir.join("subdir/c.txt").is_file());

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
    assert!(!target_dir.join("subdir").exists());
}

#[test]
fn test_unapply_mixed() {
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
            dry_run: false,
            clean_up: false,
            prune_empty_dirs: false,
        },
    )
    .unwrap();

    assert!(target_dir.join("a.txt").is_symlink());
    assert!(target_dir.join("subdir/c.txt").is_file());

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
    assert!(!target_dir.join("subdir").exists());
}

#[test]
fn test_unapply_leaves_unmanaged() {
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

    fs::write(target_dir.join("unmanaged.txt"), "keep me").unwrap();

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
    assert!(!target_dir.join("subdir").exists());
    assert!(target_dir.join("unmanaged.txt").exists());
    assert_eq!(
        fs::read_to_string(target_dir.join("unmanaged.txt")).unwrap(),
        "keep me"
    );
}

#[test]
fn test_unapply_no_prune_keeps_dirs() {
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

    assert!(target_dir.join("subdir").is_dir());

    unapply::run(
        flags(&source_dir, &target_dir),
        &db_path,
        UnapplyFlags {
            dry_run: false,
            prune_empty_dirs: false,
        },
    )
    .unwrap();

    assert!(!target_dir.join("a.txt").exists());
    assert!(!target_dir.join("subdir/c.txt").exists());
    assert!(target_dir.join("subdir").is_dir());
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
