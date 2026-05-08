use dotrift::cli::{AddFlags, GlobalFlags, OpenEditor};
use dotrift::command::add;
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
