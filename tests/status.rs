mod common;

use std::fs;
use std::os::unix::fs::symlink;

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

    let managed_source = env.path("dotfiles/config/app.conf");

    let changed_source = env.path("dotfiles/zsh/zshrc");
    fs::create_dir_all(changed_source.parent().unwrap()).unwrap();
    fs::write(&changed_source, b"original content").unwrap();

    let managed_link_source = env.path("dotfiles/editor");

    let missing_link_source = env.path("dotfiles/missing-link");
    fs::create_dir_all(missing_link_source.parent().unwrap()).unwrap();
    fs::write(&missing_link_source, b"#!/bin/sh\n").unwrap();

    let managed_file = env.path("config/app.conf");
    fs::create_dir_all(managed_file.parent().unwrap()).unwrap();
    fs::write(&managed_file, b"key=value").unwrap();

    let changed_file = env.path("zsh/zshrc");
    fs::create_dir_all(changed_file.parent().unwrap()).unwrap();
    fs::write(&changed_file, b"changed on disk").unwrap();

    let managed_link = env.path("links/editor");
    fs::create_dir_all(managed_link.parent().unwrap()).unwrap();
    symlink(&managed_link_source, &managed_link).unwrap();

    let missing_link = env.path("links/missing");
    fs::create_dir_all(missing_link.parent().unwrap()).unwrap();

    let records = [
        StateRecord {
            target_path: managed_file,
            source_path: managed_source,
            kind: Kind::File,
            content_hash: Some(hash_bytes(b"key=value")),
        },
        StateRecord {
            target_path: changed_file,
            source_path: changed_source,
            kind: Kind::File,
            content_hash: Some(hash_bytes(b"original content")),
        },
        StateRecord {
            target_path: managed_link,
            source_path: managed_link_source,
            kind: Kind::Symlink,
            content_hash: None,
        },
        StateRecord {
            target_path: missing_link,
            source_path: missing_link_source,
            kind: Kind::Symlink,
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
