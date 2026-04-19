use dotrift::cli::ApplyFlags;
use dotrift::command::apply;
use dotrift::db::Db;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

fn setup_test(config: &str) -> (TempDir, PathBuf, PathBuf, PathBuf) {
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    let target_dir = temp_dir.path().join("target");
    let db_path = temp_dir.path().join("db");

    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&target_dir).unwrap();
    fs::write(source_dir.join("dotrift.toml"), config).unwrap();
    fs::write(source_dir.join("a.txt"), "").unwrap();
    fs::write(source_dir.join("b.txt"), "").unwrap();
    fs::create_dir(source_dir.join("subdir")).unwrap();
    fs::write(source_dir.join("subdir").join("c.txt"), "").unwrap();
    fs::write(source_dir.join("subdir").join("d.txt"), "").unwrap();

    (temp_dir, source_dir, target_dir, db_path)
}

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
        source_dir.clone(),
        Some(target_dir.clone()),
        None,
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
        source_dir.clone(),
        Some(target_dir.clone()),
        None,
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
fn test_clean_up() {
    let config = r#"
[portal]
"" = ""
"#;
    let (_temp_dir, source_dir, target_dir, db_path) = setup_test(config);

    apply::run(
        source_dir.clone(),
        Some(target_dir.clone()),
        None,
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
        source_dir.clone(),
        Some(target_dir.clone()),
        None,
        &db_path,
        ApplyFlags {
            dry_run: false,
            clean_up: true,
            prune_empty_dirs: false,
        },
    )
    .unwrap();

    assert!(target_dir.join("a.txt").exists());
    assert!(target_dir.join("b.txt").exists());
    assert!(target_dir.join("subdir").exists());
    assert!(!target_dir.join("subdir/c.txt").exists());
    assert!(!target_dir.join("subdir/d.txt").exists());

    let db = Db::init(&db_path).unwrap();
    assert_eq!(db.get_all_entries().unwrap().len(), 2);
}

#[test]
fn test_clean_up_prune_empty_dirs() {
    let config = r#"
[portal]
"" = ""
"#;
    let (temp_dir, source_dir, target_dir, db_path) = setup_test(config);

    apply::run(
        source_dir.clone(),
        Some(target_dir.clone()),
        None,
        &db_path,
        ApplyFlags {
            dry_run: false,
            clean_up: false,
            prune_empty_dirs: false,
        },
    )
    .unwrap();

    fs::write(source_dir.join("dotrift.toml"), "").unwrap();

    apply::run(
        source_dir.clone(),
        Some(target_dir.clone()),
        None,
        &db_path,
        ApplyFlags {
            dry_run: false,
            clean_up: true,
            prune_empty_dirs: true,
        },
    )
    .unwrap();

    assert!(!target_dir.join("subdir").exists());
    assert!(!target_dir.exists());
    assert!(temp_dir.path().exists());
}
