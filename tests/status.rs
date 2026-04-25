use std::path::PathBuf;

use dotrift::{
    cli::ApplyFlags,
    command::{apply, status},
};
use tempfile::TempDir;

use crate::common::{flags, setup_test};

mod common;

fn setup_apply() -> (TempDir, PathBuf, PathBuf, PathBuf) {
    let tuple = setup_test(
        r#"[portal]
"" = ""
"#,
    );
    apply::run(
        flags(&tuple.1, &tuple.2),
        &tuple.3,
        ApplyFlags {
            dry_run: false,
            clean_up: false,
            prune_empty_dirs: false,
        },
    )
    .unwrap();
    tuple
}

#[test]
#[ignore]
fn test_status_list() {
    let (_tmp, _source_dir, target_dir, db_path) = setup_apply();
    status::list(Some(target_dir.join("a.txt")), &db_path).unwrap();
}

#[test]
#[ignore]
fn test_status_list_all() {
    let (_tmp, _source_dir, _target_dir, db_path) = setup_apply();
    status::list(None, &db_path).unwrap();
}

#[test]
#[ignore]
fn test_status_clear() {
    let (_tmp, _source_dir, target_dir, db_path) = setup_apply();
    status::clear(Some(target_dir.join("a.txt")), &db_path).unwrap();
}
#[test]
#[ignore]
fn test_status_clear_all() {
    let (_tmp, _source_dir, _target_dir, db_path) = setup_apply();
    status::clear(None, &db_path).unwrap();
}
