mod common;

use std::fs;

use common::{ApplyScenario, Prompt, record_of};
use dotrift::ExitStatus;
use dotrift::commands::apply::ApplyOptions;
use dotrift::state::hash_bytes;

fn dry_run_options() -> ApplyOptions {
    ApplyOptions {
        dry_run: true,
        ..Default::default()
    }
}

fn dry_run_cleanup_options() -> ApplyOptions {
    ApplyOptions {
        dry_run: true,
        clean_up: true,
        ..Default::default()
    }
}

fn dry_run_prune_options() -> ApplyOptions {
    ApplyOptions {
        dry_run: true,
        clean_up: true,
        prune_empty_dirs: true,
        ..Default::default()
    }
}

#[test]
fn dry_run_cleanup_prune_keeps_directory_with_to_be_deployed_entry() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "dir1/file1"
"#
    });

    scenario.run();
    fs::write(scenario.source.join("file2"), "content2").unwrap();
    scenario.write_config(
        r#"
[portal]
"file2" = "dir1/file2"
"#,
    );

    let output = {
        dotrift::report::clear();
        let prompter = Prompt::never();
        let status = scenario
            .try_run_with(dry_run_prune_options(), &prompter)
            .expect("apply failed");
        assert_eq!(status, ExitStatus::Success);
        dotrift::report::take_output()
    };

    assert!(
        output.contains("removed") && output.contains("dir1/file1"),
        "{output}"
    );
    assert!(
        !output.contains("pruned"),
        "dir1 gains a to-be-deployed entry; expected no prune line:\n{output}"
    );
    assert!(fs::symlink_metadata(scenario.target.join("dir1/file1")).is_ok());
}

#[test]
fn dry_run_fresh_deployment_changes_nothing() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });

    let prompter = Prompt::never();
    let status = scenario
        .try_run_with(dry_run_options(), &prompter)
        .expect("apply failed");

    assert_eq!(status, ExitStatus::Success);
    assert_eq!(prompter.calls(), 0);
    assert!(fs::symlink_metadata(scenario.target.join("file1")).is_err());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn dry_run_leaves_obstruction_untouched_without_prompt() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::write(target.join("file1"), b"content1").unwrap();
        fs::write(source.join("file2"), b"content2").unwrap();
        r#"
[portal]
"file2" = "file1"
"#
    });

    let prompter = Prompt::never();
    let status = scenario
        .try_run_with(dry_run_options(), &prompter)
        .expect("apply failed");

    assert_eq!(status, ExitStatus::Success);
    assert_eq!(prompter.calls(), 0);
    assert_eq!(
        fs::read(scenario.target.join("file1")).unwrap(),
        b"content1"
    );
    assert!(record_of(&scenario.env, &scenario.target.join("file1")).is_none());
}

#[test]
fn dry_run_leaves_tampered_copy_untouched_with_old_record() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), b"content1").unwrap();
        r#"
[portal]
"file1" = "file2"

[rule]
"file2" = { type = "copy" }
"#
    });

    scenario.run();
    fs::write(scenario.target.join("file2"), b"content2").unwrap();

    let prompter = Prompt::never();
    let status = scenario
        .try_run_with(dry_run_options(), &prompter)
        .expect("apply failed");

    assert_eq!(status, ExitStatus::Success);
    assert_eq!(prompter.calls(), 0);
    assert_eq!(
        fs::read(scenario.target.join("file2")).unwrap(),
        b"content2"
    );
    let record = record_of(&scenario.env, &scenario.target.join("file2")).expect("record missing");
    assert_eq!(
        record.content_hash.as_deref(),
        Some(hash_bytes(b"content1").as_str())
    );
}

#[test]
fn dry_run_cleanup_keeps_stale_target_and_record() {
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

    let prompter = Prompt::never();
    let status = scenario
        .try_run_with(dry_run_cleanup_options(), &prompter)
        .expect("apply failed");

    assert_eq!(status, ExitStatus::Success);
    assert_eq!(prompter.calls(), 0);
    let stale = scenario.target.join("file2");
    assert!(
        fs::symlink_metadata(&stale)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(record_of(&scenario.env, &stale).is_some());
    assert!(record_of(&scenario.env, &scenario.target.join("file1")).is_some());
}

#[test]
fn dry_run_cleanup_prune_keeps_stale_directories() {
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

    let prompter = Prompt::never();
    let status = scenario
        .try_run_with(dry_run_prune_options(), &prompter)
        .expect("apply failed");

    assert_eq!(status, ExitStatus::Success);
    assert_eq!(prompter.calls(), 0);
    let stale = scenario.target.join("dir1/sub1/file1");
    assert!(
        fs::symlink_metadata(&stale)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(record_of(&scenario.env, &stale).is_some());
    assert!(
        fs::metadata(scenario.target.join("dir1/sub1"))
            .unwrap()
            .is_dir()
    );
    assert!(fs::metadata(scenario.target.join("dir1")).unwrap().is_dir());
    assert!(record_of(&scenario.env, &scenario.target.join("file2")).is_some());
}
