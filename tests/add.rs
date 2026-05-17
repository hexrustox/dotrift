use dotrift::cli::{AddFlags, ApplyFlags, GlobalFlags, OpenEditor};
use dotrift::command::{add, apply};
use dotrift::config::DeployType;
use dotrift::db::{Db, DbEntry};
use std::fs;
use std::path::PathBuf;

use crate::common::setup_test;

mod common;

fn flags(source_dir: PathBuf) -> GlobalFlags {
    GlobalFlags::new(Some(source_dir), None, None)
}

#[test]
fn test_add() {
    let (temp_dir, source_dir, _, _) = setup_test("");

    let path = temp_dir.path().join("file");
    fs::write(&path, "").unwrap();

    add::run(
        flags(source_dir.clone()),
        path,
        Some(PathBuf::from("file")),
        AddFlags {
            copy: false,
            force: false,
            editor: Some(OpenEditor::Never),
            no_modify: false,
        },
        &temp_dir.path().join("db"),
    )
    .unwrap();

    assert!(source_dir.join("file").exists());
}

#[test]
fn test_add_reimport() {
    let (temp_dir, source_dir, _, _) = setup_test("");

    let path = temp_dir.path().join("file");
    fs::write(&path, "").unwrap();

    let db_path = &temp_dir.path().join("db");
    let db = Db::init(db_path).unwrap();
    db.insert_or_update(&DbEntry {
        target_path: path.clone(),
        deploy_type: DeployType::Copy,
        source_path: source_dir.join("file"),
        hash: None,
        symlink_target: None,
    })
    .unwrap();

    add::run(
        flags(source_dir.clone()),
        path,
        None,
        AddFlags {
            copy: false,
            force: false,
            editor: Some(OpenEditor::Never),
            no_modify: false,
        },
        db_path,
    )
    .unwrap();

    assert!(source_dir.join("file").exists());
}

#[test]
fn test_add_force_overwrite() {
    let (temp_dir, source_dir, _, _) = setup_test("");

    let path = temp_dir.path().join("new_file");
    fs::write(&path, "forced content").unwrap();

    add::run(
        flags(source_dir.clone()),
        path,
        Some(PathBuf::from("a.txt")),
        AddFlags {
            copy: false,
            force: true,
            editor: Some(OpenEditor::Never),
            no_modify: true,
        },
        &temp_dir.path().join("db"),
    )
    .unwrap();

    assert!(source_dir.join("a.txt").exists());
    assert_eq!(
        fs::read_to_string(source_dir.join("a.txt")).unwrap(),
        "forced content"
    );
}

#[test]
fn test_add_copy_flag() {
    let (temp_dir, source_dir, _, _) = setup_test("");

    let path = temp_dir.path().join("file");
    fs::write(&path, "original").unwrap();

    add::run(
        flags(source_dir.clone()),
        path.clone(),
        Some(PathBuf::from("file")),
        AddFlags {
            copy: true,
            force: false,
            editor: Some(OpenEditor::Never),
            no_modify: true,
        },
        &temp_dir.path().join("db"),
    )
    .unwrap();

    assert!(source_dir.join("file").exists());
    assert!(path.exists());
    assert_eq!(
        fs::read_to_string(source_dir.join("file")).unwrap(),
        "original"
    );
}

#[test]
fn test_add_directory_reimport() {
    let (temp_dir, source_dir, _, db_path) = setup_test("");

    let dir = temp_dir.path().join("dir");
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("x.txt"), "x").unwrap();
    fs::create_dir(dir.join("sub")).unwrap();
    fs::write(dir.join("sub/y.txt"), "y").unwrap();

    let db = Db::init(&db_path).unwrap();
    db.insert_or_update(&DbEntry {
        target_path: dir.join("x.txt"),
        deploy_type: DeployType::Symlink,
        source_path: source_dir.join("x.txt"),
        hash: None,
        symlink_target: None,
    })
    .unwrap();
    db.insert_or_update(&DbEntry {
        target_path: dir.join("sub/y.txt"),
        deploy_type: DeployType::Symlink,
        source_path: source_dir.join("sub/y.txt"),
        hash: None,
        symlink_target: None,
    })
    .unwrap();

    add::run(
        flags(source_dir.clone()),
        dir,
        None,
        AddFlags {
            copy: false,
            force: false,
            editor: Some(OpenEditor::Never),
            no_modify: true,
        },
        &db_path,
    )
    .unwrap();

    assert!(source_dir.join("x.txt").exists());
    assert_eq!(fs::read_to_string(source_dir.join("x.txt")).unwrap(), "x");
    assert!(source_dir.join("sub/y.txt").exists());
    assert_eq!(
        fs::read_to_string(source_dir.join("sub/y.txt")).unwrap(),
        "y"
    );
}

#[test]
fn test_add_nested_destination() {
    let (temp_dir, source_dir, _, _) = setup_test("");

    let path = temp_dir.path().join("extra");
    fs::write(&path, "nested").unwrap();

    add::run(
        flags(source_dir.clone()),
        path,
        Some(PathBuf::from("subdir/extra.txt")),
        AddFlags {
            copy: false,
            force: false,
            editor: Some(OpenEditor::Never),
            no_modify: true,
        },
        &temp_dir.path().join("db"),
    )
    .unwrap();

    assert!(source_dir.join("subdir/extra.txt").exists());
    assert_eq!(
        fs::read_to_string(source_dir.join("subdir/extra.txt")).unwrap(),
        "nested"
    );
}

#[test]
fn test_add_then_apply() {
    let config = r#"
[portal]
"" = ""
"#;
    let (_temp_dir, source_dir, target_dir, db_path) = setup_test(config);

    let path = _temp_dir.path().join("extra");
    fs::write(&path, "deploy_me").unwrap();

    add::run(
        flags(source_dir.clone()),
        path,
        Some(PathBuf::from("extra")),
        AddFlags {
            copy: false,
            force: false,
            editor: Some(OpenEditor::Never),
            no_modify: true,
        },
        &db_path,
    )
    .unwrap();

    assert!(source_dir.join("extra").exists());

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

    assert!(target_dir.join("extra").exists());
    assert_eq!(
        fs::read_to_string(target_dir.join("extra")).unwrap(),
        "deploy_me"
    );
}

#[test]
#[ignore]
fn test_editor_open() {
    let (temp_dir, source_dir, _, _) = setup_test("# test editor open");

    let path = temp_dir.path().join("file");
    fs::write(&path, "").unwrap();

    add::run(
        flags(source_dir.clone()),
        path,
        Some(PathBuf::from("file")),
        AddFlags {
            copy: false,
            force: false,
            editor: None,
            no_modify: false,
        },
        &temp_dir.path().join("db"),
    )
    .unwrap();

    assert!(source_dir.join("file").exists());
}

#[test]
#[ignore]
fn test_editor_command_config() {
    let (temp_dir, source_dir, _, _) = setup_test(
        r#"# test editor command config
[portal]
key = "value"

[rule]
key = {}
"#,
    );

    let config = r#"[editor-command]
command = "nano"
args = ["+{row},{col}", "{file}"]
"#;
    let config_path = temp_dir.path().join("config");
    fs::write(&config_path, config).unwrap();

    let path = temp_dir.path().join("file");
    fs::write(&path, "").unwrap();

    add::run(
        GlobalFlags::new(Some(source_dir.clone()), None, Some(config_path)),
        path,
        Some(PathBuf::from("file")),
        AddFlags {
            copy: false,
            force: false,
            editor: None,
            no_modify: false,
        },
        &temp_dir.path().join("db"),
    )
    .unwrap();

    assert!(source_dir.join("file").exists());
}

#[test]
#[ignore]
fn test_editor_missing() {
    let (temp_dir, source_dir, target_dir, _) = setup_test("");

    let path = target_dir.join("file");
    fs::write(&path, "").unwrap();

    add::run(
        GlobalFlags::new(Some(source_dir.clone()), Some(target_dir), None),
        path,
        Some(PathBuf::from("file")),
        AddFlags {
            copy: false,
            force: false,
            editor: None,
            no_modify: false,
        },
        &temp_dir.path().join("db"),
    )
    .unwrap();

    assert!(source_dir.join("file").exists());
}

#[test]
#[ignore]
fn test_editor_collision() {
    let config = r#"[portal]
"other" = "file"
"#;
    let (temp_dir, source_dir, target_dir, _) = setup_test(config);

    let path = target_dir.join("file");
    fs::write(&path, "").unwrap();
    fs::write(source_dir.join("other"), "").unwrap();

    add::run(
        GlobalFlags::new(Some(source_dir.clone()), Some(target_dir), None),
        path,
        Some(PathBuf::from("file")),
        AddFlags {
            copy: false,
            force: false,
            editor: None,
            no_modify: false,
        },
        &temp_dir.path().join("db"),
    )
    .unwrap();

    assert!(source_dir.join("file").exists());
}
