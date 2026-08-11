mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;

use common::TestEnv;
use dotrift::commands::apply;
use dotrift::state::{Kind, StateDatabase};

#[test]
fn apply_deploys_symlinks_in_target_order_and_records_only_files() {
    let env = TestEnv::new();
    let source = env.path("source");
    let target = env.path("target");
    fs::create_dir_all(source.join("nested")).expect("create source");
    fs::write(source.join("z"), b"z").expect("write z");
    fs::write(source.join("a"), b"a").expect("write a");
    fs::write(source.join("nested/b"), b"b").expect("write b");
    fs::write(
        source.join("dotrift.toml"),
        "[portal]\n\"z\" = \"nested/z\"\n\"a\" = \"a\"\n\"nested/b\" = \"nested/b\"\n",
    )
    .expect("write config");

    apply::run(&source, Some(target.clone())).expect("apply deployment");

    assert_eq!(
        fs::read_link(target.join("a")).expect("read a link"),
        source.join("a")
    );
    assert_eq!(
        fs::read_link(target.join("nested/b")).expect("read b link"),
        source.join("nested/b")
    );
    assert_eq!(
        fs::read_link(target.join("nested/z")).expect("read z link"),
        source.join("z")
    );

    let records = StateDatabase::open()
        .expect("open state")
        .managed_paths()
        .expect("read state");
    assert_eq!(records.len(), 3);
    assert!(records.iter().all(|record| record.kind == Kind::Symlink));
    assert!(
        !records
            .iter()
            .any(|record| record.target_path == target.join("nested"))
    );
}

#[test]
fn apply_replaces_a_clean_managed_symlink() {
    let env = TestEnv::new();
    let source = env.path("source");
    let target = env.path("target");
    fs::create_dir_all(&source).expect("create source");
    fs::write(source.join("first"), b"first").expect("write first");
    fs::write(source.join("second"), b"second").expect("write second");
    fs::write(
        source.join("dotrift.toml"),
        "[portal]\n\"first\" = \"out\"\n",
    )
    .expect("write config");

    apply::run(&source, Some(target.clone())).expect("initial apply");
    fs::write(
        source.join("dotrift.toml"),
        "[portal]\n\"second\" = \"out\"\n",
    )
    .expect("update config");
    apply::run(&source, Some(target.clone())).expect("replacement apply");

    assert_eq!(
        fs::read_link(target.join("out")).expect("read link"),
        source.join("second")
    );
    let records = StateDatabase::open()
        .expect("open state")
        .managed_paths()
        .expect("read state");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].source_path, source.join("second"));
}

#[test]
fn apply_skips_an_unmanaged_obstruction_and_keeps_it_unmodified() {
    let env = TestEnv::new();
    let source = env.path("source");
    let target = env.path("target");
    fs::create_dir_all(&source).expect("create source");
    fs::write(source.join("file"), b"desired").expect("write source");
    fs::write(
        source.join("dotrift.toml"),
        "[portal]\n\"file\" = \"out\"\n",
    )
    .expect("write config");
    fs::create_dir_all(&target).expect("create target");
    fs::write(target.join("out"), b"keep me").expect("write obstruction");

    assert!(apply::run(&source, Some(target.clone())).is_err());
    assert_eq!(
        fs::read(target.join("out")).expect("read obstruction"),
        b"keep me"
    );
    assert!(
        env.database()
            .record(&target.join("out"))
            .expect("read state")
            .is_none()
    );
}

#[test]
fn apply_skipping_a_parent_obstruction_does_not_create_earlier_parents() {
    let env = TestEnv::new();
    let source = env.path("source");
    let target = env.path("target");
    fs::create_dir_all(&source).expect("create source");
    fs::write(source.join("file"), b"desired").expect("write source");
    fs::write(
        source.join("dotrift.toml"),
        "[portal]\n\"file\" = \"new/blocked/out\"\n",
    )
    .expect("write config");
    fs::create_dir_all(target.join("new")).expect("create parent");
    fs::write(target.join("new/blocked"), b"keep me").expect("write obstruction");

    assert!(apply::run(&source, Some(target.clone())).is_err());
    assert!(!target.join("new/blocked/out").exists());
    assert_eq!(
        fs::read(target.join("new/blocked")).expect("read obstruction"),
        b"keep me"
    );
}

#[test]
fn apply_does_not_traverse_a_symlink_parent() {
    let env = TestEnv::new();
    let source = env.path("source");
    let target = env.path("target");
    let outside = env.path("outside");
    fs::create_dir_all(&source).expect("create source");
    fs::create_dir_all(&outside).expect("create outside");
    fs::write(source.join("file"), b"file").expect("write source");
    fs::write(
        source.join("dotrift.toml"),
        "[portal]\n\"file\" = \"nested/file\"\n",
    )
    .expect("write config");
    fs::create_dir_all(&target).expect("create target");
    std::os::unix::fs::symlink(&outside, target.join("nested")).expect("create parent link");

    assert!(apply::run(&source, Some(target.clone())).is_err());
    assert!(!outside.join("file").exists());
}

#[test]
fn apply_copies_bytes_records_fingerprint_and_applies_mode() {
    let env = TestEnv::new();
    let source = env.path("source");
    let target = env.path("target");
    fs::create_dir_all(&source).expect("create source");
    fs::write(source.join("file"), b"copy me").expect("write source");
    fs::write(
        source.join("dotrift.toml"),
        "[portal]\n\"file\" = \"out\"\n[rule]\n\"out\" = { type = \"copy\", mode = \"600\" }\n",
    )
    .expect("write config");

    apply::run(&source, Some(target.clone())).expect("apply deployment");

    assert_eq!(
        fs::read(target.join("out")).expect("read output"),
        b"copy me"
    );
    assert_eq!(
        fs::metadata(target.join("out"))
            .expect("stat output")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let record = env
        .database()
        .record(&target.join("out"))
        .expect("read state")
        .expect("state record");
    assert_eq!(record.kind, Kind::File);
    assert_eq!(record.content_hash.as_deref(), Some("e665290c3bf08e31"));
}

#[test]
fn apply_renders_template_with_resolved_variables_before_writing() {
    let env = TestEnv::new();
    let source = env.path("source");
    let target = env.path("target");
    fs::create_dir_all(&source).expect("create source");
    fs::write(source.join("file"), b"hello {{ name }}").expect("write source");
    fs::write(
        source.join("dotrift_data.toml"),
        "[variable]\nname = \"profile\"\n",
    )
    .expect("write data");
    fs::write(
        source.join("dotrift.toml"),
        "[portal]\n\"file\" = \"out\"\n[rule]\n\"out\" = { type = \"template\" }\n",
    )
    .expect("write config");

    apply::run(&source, Some(target.clone())).expect("apply deployment");

    assert_eq!(
        fs::read(target.join("out")).expect("read output"),
        b"hello profile"
    );
    let record = env
        .database()
        .record(&target.join("out"))
        .expect("read state")
        .expect("state record");
    assert_eq!(record.content_hash.as_deref(), Some("4855d9d542918ce5"));
}
