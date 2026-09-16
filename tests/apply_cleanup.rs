mod common;

use std::fs;
use std::os::unix::fs::symlink;

use common::{ApplyScenario, Prompt, record_of};
use dotrift::ExitStatus;
use dotrift::commands::apply::ApplyOptions;
use dotrift::deploy::ObstructionChoice;
use dotrift::state::{Kind, StateRecord, hash_bytes};

fn cleanup_options() -> ApplyOptions {
    ApplyOptions {
        clean_up: true,
        ..Default::default()
    }
}

fn prune_options() -> ApplyOptions {
    ApplyOptions {
        clean_up: true,
        prune_empty_dirs: true,
        ..Default::default()
    }
}

#[test]
fn cleanup_removes_stale_symlink() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "file2"
"#
    });

    scenario.run();
    scenario.write_config(
        r#"
[portal]
"file1" = "file1"
"#,
    );
    scenario.run_with_options(cleanup_options());

    assert!(fs::symlink_metadata(scenario.target.join("file2")).is_err());
    assert!(record_of(&scenario.env, &scenario.target.join("file2")).is_none());
    let link = scenario.target.join("file1");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(record_of(&scenario.env, &link).is_some());
}

#[test]
fn cleanup_removes_stale_copy() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "file2"

[rule]
"file2" = { type = "copy" }
"#
    });

    scenario.run();
    scenario.write_config(
        r#"
[portal]
"file1" = "file1"
"#,
    );
    scenario.run_with_options(cleanup_options());

    assert!(fs::symlink_metadata(scenario.target.join("file2")).is_err());
    assert!(record_of(&scenario.env, &scenario.target.join("file2")).is_none());
    assert_eq!(
        fs::read(scenario.target.join("file1")).unwrap(),
        b"content1"
    );
    assert!(record_of(&scenario.env, &scenario.target.join("file1")).is_some());
}

#[test]
fn cleanup_removes_stale_template() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "{{ str }}\n").unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "file2"

[rule]
"file2" = { type = "template" }
"#
    });
    scenario.env.write_data_file("[variable]\nstr = \"str\"\n");

    scenario.run();
    assert_eq!(fs::read(scenario.target.join("file2")).unwrap(), b"str\n");
    scenario.write_config(
        r#"
[portal]
"file1" = "file1"
"#,
    );
    scenario.run_with_options(cleanup_options());

    assert!(fs::symlink_metadata(scenario.target.join("file2")).is_err());
    assert!(record_of(&scenario.env, &scenario.target.join("file2")).is_none());
    assert!(
        fs::symlink_metadata(scenario.target.join("file1"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn cleanup_removes_dir_portal_subtree() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::create_dir_all(source.join("dir1/sub1")).unwrap();
        fs::write(source.join("dir1/file2"), "content2").unwrap();
        fs::write(source.join("dir1/sub1/file3"), "content3").unwrap();
        r#"
[portal]
"file1" = "file1"
"dir1" = "dir1"
"#
    });

    scenario.run();
    assert_eq!(
        fs::read(scenario.target.join("dir1/sub1/file3")).unwrap(),
        b"content3"
    );
    scenario.write_config(
        r#"
[portal]
"file1" = "file1"
"#,
    );
    scenario.run_with_options(cleanup_options());

    assert!(fs::symlink_metadata(scenario.target.join("dir1/file2")).is_err());
    assert!(fs::symlink_metadata(scenario.target.join("dir1/sub1/file3")).is_err());
    assert!(record_of(&scenario.env, &scenario.target.join("dir1/file2")).is_none());
    assert!(record_of(&scenario.env, &scenario.target.join("dir1/sub1/file3")).is_none());
    assert!(
        fs::symlink_metadata(scenario.target.join("file1"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(record_of(&scenario.env, &scenario.target.join("file1")).is_some());
}

#[test]
fn cleanup_keeps_desired_while_removing_stale() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        fs::write(source.join("file3"), "content3").unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "file2"
"file3" = "file3"
"#
    });

    scenario.run();
    scenario.write_config(
        r#"
[portal]
"file1" = "file1"
"file2" = "file2"
"#,
    );
    scenario.run_with_options(cleanup_options());

    assert!(fs::symlink_metadata(scenario.target.join("file3")).is_err());
    assert!(record_of(&scenario.env, &scenario.target.join("file3")).is_none());
    for name in ["file1", "file2"] {
        let link = scenario.target.join(name);
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "{name} should still be deployed"
        );
        assert!(record_of(&scenario.env, &link).is_some());
    }
}

#[test]
fn cleanup_without_stale_entries_is_noop() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });

    scenario.run();
    scenario.run_with_options(cleanup_options());

    let link = scenario.target.join("file1");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&link).unwrap(), b"content1");
    assert!(record_of(&scenario.env, &link).is_some());
}

#[test]
fn cleanup_relinquishes_tampered_stale_copy() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "file2"

[rule]
"file2" = { type = "copy" }
"#
    });

    scenario.run();
    fs::write(scenario.target.join("file2"), "content1").unwrap();
    scenario.write_config(
        r#"
[portal]
"file1" = "file1"
"#,
    );
    scenario.run_with_options(cleanup_options());

    assert_eq!(
        fs::read(scenario.target.join("file2")).unwrap(),
        b"content1"
    );
    assert!(record_of(&scenario.env, &scenario.target.join("file2")).is_none());
    assert!(
        fs::symlink_metadata(scenario.target.join("file1"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn cleanup_relinquishes_missing_stale_target() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "file2"
"#
    });

    scenario.run();
    fs::remove_file(scenario.target.join("file2")).unwrap();
    scenario.write_config(
        r#"
[portal]
"file1" = "file1"
"#,
    );
    scenario.run_with_options(cleanup_options());

    assert!(fs::symlink_metadata(scenario.target.join("file2")).is_err());
    assert!(record_of(&scenario.env, &scenario.target.join("file2")).is_none());
    assert!(
        fs::symlink_metadata(scenario.target.join("file1"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(record_of(&scenario.env, &scenario.target.join("file1")).is_some());
}

#[test]
fn cleanup_relinquishes_children_when_dir_replaced_by_file() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::create_dir_all(source.join("dir1/sub1")).unwrap();
        fs::write(source.join("dir1/file2"), "content2").unwrap();
        fs::write(source.join("dir1/sub1/file3"), "content3").unwrap();
        r#"
[portal]
"file1" = "file1"
"dir1" = "dir1"
"#
    });

    scenario.run();
    fs::remove_dir_all(scenario.target.join("dir1")).unwrap();
    fs::write(scenario.target.join("dir1"), "content1").unwrap();
    scenario.write_config(
        r#"
[portal]
"file1" = "file1"
"#,
    );
    scenario.run_with_options(cleanup_options());

    assert_eq!(fs::read(scenario.target.join("dir1")).unwrap(), b"content1");
    assert!(record_of(&scenario.env, &scenario.target.join("dir1/file2")).is_none());
    assert!(record_of(&scenario.env, &scenario.target.join("dir1/sub1/file3")).is_none());
    assert!(record_of(&scenario.env, &scenario.target.join("file1")).is_some());
}

#[test]
fn cleanup_relinquishes_file_replaced_by_dir() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "file2"
"#
    });

    scenario.run();
    fs::remove_file(scenario.target.join("file1")).unwrap();
    fs::create_dir_all(scenario.target.join("file1/dir1")).unwrap();
    fs::write(scenario.target.join("file1/dir1/file2"), "content2").unwrap();
    scenario.write_config(
        r#"
[portal]
"file2" = "file2"
"#,
    );
    scenario.run_with_options(cleanup_options());

    assert!(
        fs::metadata(scenario.target.join("file1"))
            .unwrap()
            .is_dir()
    );
    assert_eq!(
        fs::read(scenario.target.join("file1/dir1/file2")).unwrap(),
        b"content2"
    );
    assert!(record_of(&scenario.env, &scenario.target.join("file1")).is_none());
    assert!(
        fs::symlink_metadata(scenario.target.join("file2"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn prune_removes_emptied_parent_chain() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        r#"
[portal]
"file1" = "dir1/sub1/file1"
"file2" = "file2"
"#
    });

    scenario.run();
    assert!(scenario.target.join("dir1/sub1/file1").exists());
    scenario.write_config(
        r#"
[portal]
"file2" = "file2"
"#,
    );
    scenario.run_with_options(prune_options());

    assert!(fs::symlink_metadata(scenario.target.join("dir1/sub1/file1")).is_err());
    assert!(!scenario.target.join("dir1/sub1").exists());
    assert!(!scenario.target.join("dir1").exists());
    assert!(record_of(&scenario.env, &scenario.target.join("file2")).is_some());
}

#[test]
fn cleanup_without_prune_keeps_empty_parents() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        r#"
[portal]
"file1" = "dir1/sub1/file1"
"file2" = "file2"
"#
    });

    scenario.run();
    scenario.write_config(
        r#"
[portal]
"file2" = "file2"
"#,
    );
    scenario.run_with_options(cleanup_options());

    assert!(fs::symlink_metadata(scenario.target.join("dir1/sub1/file1")).is_err());
    assert!(
        fs::metadata(scenario.target.join("dir1/sub1"))
            .unwrap()
            .is_dir()
    );
    assert!(fs::metadata(scenario.target.join("dir1")).unwrap().is_dir());
}

#[test]
fn prune_stops_at_parent_holding_desired_target() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        r#"
[portal]
"file1" = "dir1/sub1/file1"
"file2" = "dir1/file2"
"#
    });

    scenario.run();
    scenario.write_config(
        r#"
[portal]
"file2" = "dir1/file2"
"#,
    );
    scenario.run_with_options(prune_options());

    assert!(fs::symlink_metadata(scenario.target.join("dir1/sub1/file1")).is_err());
    assert!(!scenario.target.join("dir1/sub1").exists());
    assert!(fs::metadata(scenario.target.join("dir1")).unwrap().is_dir());
    assert_eq!(
        fs::read(scenario.target.join("dir1/file2")).unwrap(),
        b"content2"
    );
    assert!(record_of(&scenario.env, &scenario.target.join("dir1/file2")).is_some());
}

#[test]
fn prune_stops_at_parent_holding_unmanaged_subdir() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        r#"
[portal]
"file1" = "dir1/sub1/file1"
"file2" = "file2"
"#
    });

    scenario.run();
    fs::create_dir_all(scenario.target.join("dir1/dir2")).unwrap();
    fs::write(scenario.target.join("dir1/dir2/file2"), "content2").unwrap();
    scenario.write_config(
        r#"
[portal]
"file2" = "file2"
"#,
    );
    scenario.run_with_options(prune_options());

    assert!(!scenario.target.join("dir1/sub1").exists());
    assert!(fs::metadata(scenario.target.join("dir1")).unwrap().is_dir());
    assert_eq!(
        fs::read(scenario.target.join("dir1/dir2/file2")).unwrap(),
        b"content2"
    );
    assert!(record_of(&scenario.env, &scenario.target.join("dir1/dir2/file2")).is_none());
}

#[test]
fn prune_stops_at_symlink_parent() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::create_dir_all(target.join("dir1")).unwrap();
        symlink("dir1", target.join("link1")).unwrap();
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        r#"
[portal]
"file1" = "link1/file1"
"file2" = "file2"
"#
    });

    scenario.run();
    assert_eq!(
        fs::read(scenario.target.join("dir1/file1")).unwrap(),
        b"content1"
    );
    scenario.write_config(
        r#"
[portal]
"file2" = "file2"
"#,
    );
    scenario.run_with_options(prune_options());

    assert!(fs::symlink_metadata(scenario.target.join("link1/file1")).is_err());
    assert!(
        fs::symlink_metadata(scenario.target.join("link1"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(fs::metadata(scenario.target.join("dir1")).unwrap().is_dir());
}

#[test]
fn cleanup_unlinks_stale_file_behind_symlink_parent() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::create_dir_all(target.join("dir1")).unwrap();
        symlink("dir1", target.join("link1")).unwrap();
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        r#"
[portal]
"file1" = "link1/file1"
"file2" = "file2"
"#
    });

    scenario.run();
    scenario.write_config(
        r#"
[portal]
"file2" = "file2"
"#,
    );
    scenario.run_with_options(cleanup_options());

    assert!(fs::symlink_metadata(scenario.target.join("link1/file1")).is_err());
    assert!(!scenario.target.join("dir1/file1").exists());
    assert_eq!(
        fs::read_link(scenario.target.join("link1")).unwrap(),
        std::path::Path::new("dir1")
    );
    assert!(fs::metadata(scenario.target.join("dir1")).unwrap().is_dir());
    assert!(record_of(&scenario.env, &scenario.target.join("link1/file1")).is_none());
}

#[test]
fn skipped_entry_blocks_cleanup_of_stale_entries() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "file2"

[rule]
"file1" = { type = "copy" }
"file2" = { type = "copy" }
"#
    });

    scenario.run();
    fs::write(scenario.target.join("file1"), "content2").unwrap();
    scenario.write_config(
        r#"
[portal]
"file1" = "file1"
"#,
    );

    let prompter = Prompt::once(ObstructionChoice::Skip);
    let status = scenario
        .try_run_with_options_and_prompter(cleanup_options(), &prompter)
        .expect("apply failed");

    assert_eq!(status, ExitStatus::Skipped);
    assert_eq!(prompter.calls(), 1);
    assert_eq!(
        fs::read(scenario.target.join("file1")).unwrap(),
        b"content2"
    );
    assert_eq!(
        fs::read(scenario.target.join("file2")).unwrap(),
        b"content2"
    );
    assert!(record_of(&scenario.env, &scenario.target.join("file2")).is_some());
}

#[test]
fn cleanup_ignores_records_outside_target_root() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "file2"
"#
    });

    scenario.run();
    let outside_dir = scenario.env.path("dir1");
    fs::create_dir_all(&outside_dir).unwrap();
    let outside_path = outside_dir.join("file1");
    fs::write(&outside_path, "content1").unwrap();
    scenario
        .env
        .database()
        .put(&StateRecord {
            target_path: outside_path.clone(),
            source_path: scenario.source.join("file1"),
            kind: Kind::File,
            content_hash: Some(hash_bytes(b"content1").into()),
        })
        .unwrap();

    scenario.write_config(
        r#"
[portal]
"file1" = "file1"
"#,
    );
    scenario.run_with_options(cleanup_options());

    assert_eq!(fs::read(&outside_path).unwrap(), b"content1");
    assert!(record_of(&scenario.env, &outside_path).is_some());
    assert!(fs::symlink_metadata(scenario.target.join("file2")).is_err());
    assert!(
        fs::symlink_metadata(scenario.target.join("file1"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn prune_to_empty_keeps_target_root() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        r#"
[portal]
"file1" = "dir1/sub1/file1"
"file2" = "dir1/file2"
"#
    });

    scenario.run();
    scenario.write_config("[portal]\n");
    scenario.run_with_options(prune_options());

    assert!(fs::symlink_metadata(scenario.target.join("dir1/sub1/file1")).is_err());
    assert!(fs::symlink_metadata(scenario.target.join("dir1/file2")).is_err());
    assert!(fs::metadata(&scenario.target).unwrap().is_dir());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}
