mod common;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

use common::TestEnv;
use dotrift::hash::hash_bytes;
use dotrift::state::{Kind, StateRecord};

fn run_status_and_take() -> String {
    dotrift::capture::clear();
    dotrift::commands::status::run().expect("status run failed");
    dotrift::capture::take()
}

#[test]
fn missing_database_prints_nothing() {
    let _env = TestEnv::new();
    assert_eq!(run_status_and_take(), "");
}

#[test]
fn empty_database_prints_nothing() {
    let env = TestEnv::new();
    let _database = env.database();
    assert_eq!(run_status_and_take(), "");
}

#[test]
fn prints_sorted_lines_with_verdicts() {
    let env = TestEnv::new();
    let database = env.database();

    let managed_file = env.path("config/app.conf");
    fs::create_dir_all(managed_file.parent().unwrap()).unwrap();
    fs::write(&managed_file, b"key=value").unwrap();

    let changed_file = env.path("zsh/zshrc");
    fs::create_dir_all(changed_file.parent().unwrap()).unwrap();
    fs::write(&changed_file, b"changed on disk").unwrap();

    let managed_link = env.path("links/editor");
    fs::create_dir_all(managed_link.parent().unwrap()).unwrap();
    symlink(env.path("links/elsewhere"), &managed_link).unwrap();

    let missing_link = env.path("links/missing");
    fs::create_dir_all(missing_link.parent().unwrap()).unwrap();

    let records = [
        StateRecord {
            target_path: managed_file,
            source_path: PathBuf::from("dotfiles/config/app.conf"),
            kind: Kind::File,
            link_target: None,
            content_hash: Some(hash_bytes(b"key=value")),
        },
        StateRecord {
            target_path: changed_file,
            source_path: PathBuf::from("dotfiles/zsh/zshrc"),
            kind: Kind::File,
            link_target: None,
            content_hash: Some(hash_bytes(b"original content")),
        },
        StateRecord {
            target_path: managed_link,
            source_path: PathBuf::from("dotfiles/links/editor"),
            kind: Kind::Symlink,
            link_target: Some(env.path("links/elsewhere")),
            content_hash: None,
        },
        StateRecord {
            target_path: missing_link,
            source_path: PathBuf::from("dotfiles/links/missing"),
            kind: Kind::Symlink,
            link_target: Some(env.path("links/elsewhere")),
            content_hash: None,
        },
    ];
    for record in &records {
        database.put(record).unwrap();
    }

    let captured = run_status_and_take();

    let mut settings = insta::Settings::clone_current();
    settings.add_filter(env.root().to_str().unwrap(), "<root>");
    settings.bind(|| {
        insta::assert_snapshot!(captured);
    });
}
