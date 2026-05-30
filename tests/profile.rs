use std::{fs, path::PathBuf};

use dotrift::{cli::GlobalFlags, command::profile};
use tempfile::TempDir;

fn setup_profile(data_toml: &str) -> (TempDir, PathBuf, PathBuf) {
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    let db_path = temp_dir.path().join("db");

    fs::create_dir(&source_dir).unwrap();
    if !data_toml.is_empty() {
        fs::write(source_dir.join("dotrift_data.toml"), data_toml).unwrap();
    }

    (temp_dir, source_dir, db_path)
}

fn flags(source_dir: &std::path::Path) -> GlobalFlags {
    GlobalFlags::new(Some(source_dir.to_path_buf()), None, None)
}

#[test]
#[ignore]
fn test_profile_list_empty() {
    let (_tmp, _source_dir, db_path) = setup_profile("");
    let _ = profile::list(&flags(&_source_dir), &db_path);
}

#[test]
#[ignore]
fn test_profile_list_no_active() {
    let data = r#"
[profile.work]
email = "work@example.com"

[profile.personal]
email = "personal@example.com"
"#;
    let (_tmp, source_dir, db_path) = setup_profile(data);
    let _ = profile::list(&flags(&source_dir), &db_path);
}

#[test]
#[ignore]
fn test_profile_list_active() {
    let data = r#"
[profile.work]
email = "work@example.com"

[profile.personal]
email = "personal@example.com"
"#;
    let (_tmp, source_dir, db_path) = setup_profile(data);
    let _ = profile::activate(&flags(&source_dir), &db_path, "work");
    let _ = profile::list(&flags(&source_dir), &db_path);
}

#[test]
#[ignore]
fn test_profile_activate_valid() {
    let data = r#"
[profile.work]
email = "work@example.com"
"#;
    let (_tmp, source_dir, db_path) = setup_profile(data);
    let _ = profile::activate(&flags(&source_dir), &db_path, "work");
}

#[test]
#[ignore]
fn test_profile_activate_invalid() {
    let data = r#"
[profile.work]
email = "work@example.com"
"#;
    let (_tmp, source_dir, db_path) = setup_profile(data);
    let _ = profile::activate(&flags(&source_dir), &db_path, "nope");
}

#[test]
#[ignore]
fn test_profile_deactivate() {
    let data = r#"
[profile.work]
email = "work@example.com"
"#;
    let (_tmp, source_dir, db_path) = setup_profile(data);
    let _ = profile::activate(&flags(&source_dir), &db_path, "work");
    let _ = profile::deactivate(&db_path, "work");
}

#[test]
#[ignore]
fn test_profile_deactivate_nonexistent() {
    let (_tmp, _source_dir, db_path) = setup_profile("");
    let _ = profile::deactivate(&db_path, "nope");
}

#[test]
#[ignore]
fn test_profile_show_empty() {
    let (_tmp, _source_dir, db_path) = setup_profile("");
    let _ = profile::show(&flags(&_source_dir), &db_path);
}

#[test]
#[ignore]
fn test_profile_show_base_only() {
    let data = r#"
[variable]
name = "Alice"
age = 30
"#;
    let (_tmp, source_dir, db_path) = setup_profile(data);
    let _ = profile::show(&flags(&source_dir), &db_path);
}

#[test]
#[ignore]
fn test_profile_show_with_profiles() {
    let data = r#"
[variable]
name = "Alice"
editor = "nano"

[profile.work]
email = "work@example.com"

[profile.personal]
editor = "vim"
gh_user = "me"
"#;
    let (_tmp, source_dir, db_path) = setup_profile(data);
    let _ = profile::activate(&flags(&source_dir), &db_path, "work");
    let _ = profile::activate(&flags(&source_dir), &db_path, "personal");
    let _ = profile::show(&flags(&source_dir), &db_path);
}
