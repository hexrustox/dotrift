mod common;

use std::fs;
use std::os::unix::fs::symlink;

use dotrift::state::hash_bytes;
use dotrift::state::{Kind, StateRecord};

use common::{TestEnv, snapshot_settings};

fn run_status_and_take(env: &TestEnv) -> String {
    run_colored_status_and_take(env, false)
}

fn run_colored_status_and_take(env: &TestEnv, color: bool) -> String {
    dotrift::report::clear();
    dotrift::commands::status::run(env.env(), color).expect("status run failed");
    dotrift::report::take_output()
}

#[test]
fn status_reports_nothing_without_database() {
    let env = TestEnv::new();
    assert_eq!(run_status_and_take(&env), "");
}

#[test]
fn status_reports_nothing_for_empty_database() {
    let env = TestEnv::new();
    let _database = env.database();
    assert_eq!(run_status_and_take(&env), "");
}

#[test]
fn status_prints_sorted_lines_with_verdicts() {
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
            content_hash: Some(String::from(hash_bytes(b"key=value"))),
        },
        StateRecord {
            target_path: changed_file,
            source_path: changed_source,
            kind: Kind::File,
            content_hash: Some(String::from(hash_bytes(b"original content"))),
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

    let captured = run_status_and_take(&env);
    snapshot_settings(&env).bind(|| {
        insta::assert_snapshot!(captured);
    });
}

#[test]
fn status_layout_is_unchanged_with_color_forced() {
    let env = TestEnv::new();
    let database = env.database();

    let managed_source = env.path("dotfiles/app.conf");
    let managed_target = env.path("config/app.conf");
    fs::create_dir_all(managed_target.parent().unwrap()).unwrap();
    fs::write(&managed_target, b"key=value").unwrap();

    let changed_source = env.path("dotfiles/zsh/zshrc");
    let changed_target = env.path("zsh/zshrc");
    fs::create_dir_all(changed_target.parent().unwrap()).unwrap();
    fs::write(&changed_target, b"changed on disk").unwrap();

    for (source_path, target_path, content_hash) in [
        (
            &managed_source,
            &managed_target,
            Some(String::from(hash_bytes(b"key=value"))),
        ),
        (
            &changed_source,
            &changed_target,
            Some(String::from(hash_bytes(b"original content"))),
        ),
    ] {
        database
            .put(&StateRecord {
                target_path: target_path.clone(),
                source_path: source_path.clone(),
                kind: Kind::File,
                content_hash,
            })
            .unwrap();
    }

    // One self-check: the run must actually be colored, or the snapshot
    // below would pass against colorless output.
    let colored = run_colored_status_and_take(&env, true);
    assert!(
        colored.contains('\u{1b}'),
        "color forcing must reach the output"
    );

    // The stored snapshot, escaped codes stripped, is the colorless layout:
    // its column positions must match `status_prints_sorted_lines_with_verdicts`.
    let mut settings = snapshot_settings(&env);
    settings.set_strip_ansi_escape_codes(true);
    settings.bind(|| {
        insta::assert_snapshot!(colored);
    });
}
