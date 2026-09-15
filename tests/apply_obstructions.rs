mod common;

use std::fs;
use std::os::unix::fs::symlink;

use common::{ApplyScenario, Prompt, record_of};
use dotrift::ExitStatus;
use dotrift::deploy::ObstructionChoice;
use dotrift::state::{Kind, hash_bytes};

#[test]
fn skipping_unmanaged_target_keeps_file_and_reports_skipped() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::write(target.join("file1"), b"content1").unwrap();
        fs::write(source.join("file2"), b"content2").unwrap();
        r#"
[portal]
"file2" = "file1"
"#
    });

    let prompter = Prompt::once(ObstructionChoice::Skip);
    let status = scenario
        .try_run_with_prompter(&prompter)
        .expect("apply failed");

    assert_eq!(status, ExitStatus::Skipped);
    assert_eq!(prompter.calls(), 1);
    assert_eq!(
        fs::read(scenario.target.join("file1")).unwrap(),
        b"content1"
    );
    assert!(record_of(&scenario.env, &scenario.target.join("file1")).is_none());
}

#[test]
fn replacing_unmanaged_target_deploys_and_records() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::write(target.join("file1"), b"content1").unwrap();
        fs::write(source.join("file2"), b"content2").unwrap();
        r#"
[portal]
"file2" = "file1"
"#
    });

    let prompter = Prompt::once(ObstructionChoice::Replace);
    let status = scenario
        .try_run_with_prompter(&prompter)
        .expect("apply failed");

    assert_eq!(status, ExitStatus::Success);
    assert_eq!(prompter.calls(), 1);
    let link = scenario.target.join("file1");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&link).unwrap(), b"content2");
    let record = record_of(&scenario.env, &link).expect("no state record after replace");
    assert_eq!(record.kind, Kind::Symlink);
    assert_eq!(record.source_path, scenario.source.join("file2"));
}

#[test]
fn replace_all_latches_without_reprompting() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::write(target.join("file1"), b"content1").unwrap();
        fs::write(target.join("file2"), b"content3").unwrap();
        fs::write(source.join("file1"), b"content2").unwrap();
        fs::write(source.join("file2"), b"content4").unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "file2"
"#
    });

    let prompter = Prompt::once(ObstructionChoice::ReplaceAll);
    let status = scenario
        .try_run_with_prompter(&prompter)
        .expect("apply failed");

    assert_eq!(status, ExitStatus::Success);
    assert_eq!(prompter.calls(), 1);
    assert_eq!(
        fs::read(scenario.target.join("file1")).unwrap(),
        b"content2"
    );
    assert_eq!(
        fs::read(scenario.target.join("file2")).unwrap(),
        b"content4"
    );
    assert!(record_of(&scenario.env, &scenario.target.join("file1")).is_some());
    assert!(record_of(&scenario.env, &scenario.target.join("file2")).is_some());
}

#[test]
fn cancelling_second_prompt_preserves_completed_entry() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::write(source.join("file1"), b"content1").unwrap();
        fs::write(source.join("file2"), b"content2").unwrap();
        fs::write(target.join("file2"), b"content3").unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "file2"
"#
    });

    let prompter = Prompt::cancel();
    let status = scenario
        .try_run_with_prompter(&prompter)
        .expect("apply failed");

    assert_eq!(status, ExitStatus::Cancelled);
    assert_eq!(prompter.calls(), 1);
    assert_eq!(
        fs::read(scenario.target.join("file1")).unwrap(),
        b"content1"
    );
    assert!(record_of(&scenario.env, &scenario.target.join("file1")).is_some());
    assert_eq!(
        fs::read(scenario.target.join("file2")).unwrap(),
        b"content3"
    );
    assert!(record_of(&scenario.env, &scenario.target.join("file2")).is_none());
}

#[test]
fn skipping_tampered_copy_retains_tamper_and_old_hash() {
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

    let prompter = Prompt::once(ObstructionChoice::Skip);
    let status = scenario
        .try_run_with_prompter(&prompter)
        .expect("apply failed");

    assert_eq!(status, ExitStatus::Skipped);
    assert_eq!(prompter.calls(), 1);
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
fn replacing_tampered_copy_restores_content_and_hash() {
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

    let prompter = Prompt::once(ObstructionChoice::Replace);
    let status = scenario
        .try_run_with_prompter(&prompter)
        .expect("apply failed");

    assert_eq!(status, ExitStatus::Success);
    assert_eq!(prompter.calls(), 1);
    assert_eq!(
        fs::read(scenario.target.join("file2")).unwrap(),
        b"content1"
    );
    let record = record_of(&scenario.env, &scenario.target.join("file2")).expect("record missing");
    assert_eq!(
        record.content_hash.as_deref(),
        Some(hash_bytes(b"content1").as_str())
    );
}

#[test]
fn replacing_tampered_symlink_restores_link_and_kind() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), b"content1").unwrap();
        r#"
[portal]
"file1" = "file2"
"#
    });
    scenario.run();
    fs::remove_file(scenario.target.join("file2")).unwrap();
    fs::write(scenario.target.join("file2"), b"content2").unwrap();

    let prompter = Prompt::once(ObstructionChoice::Replace);
    let status = scenario
        .try_run_with_prompter(&prompter)
        .expect("apply failed");

    assert_eq!(status, ExitStatus::Success);
    assert_eq!(prompter.calls(), 1);
    let link = scenario.target.join("file2");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_link(&link).unwrap(), scenario.source.join("file1"));
    let record = record_of(&scenario.env, &link).expect("no state record after replace");
    assert_eq!(record.kind, Kind::Symlink);
}

#[test]
fn skipping_parent_obstruction_leaves_blocker() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::create_dir_all(target.join("dir1")).unwrap();
        fs::write(target.join("dir1/sub1"), b"content1").unwrap();
        fs::write(source.join("file1"), b"content2").unwrap();
        r#"
[portal]
"file1" = "dir1/sub1/file2"
"#
    });

    let prompter = Prompt::once(ObstructionChoice::Skip);
    let status = scenario
        .try_run_with_prompter(&prompter)
        .expect("apply failed");

    assert_eq!(status, ExitStatus::Skipped);
    assert_eq!(prompter.calls(), 1);
    assert_eq!(
        fs::read(scenario.target.join("dir1/sub1")).unwrap(),
        b"content1"
    );
    assert!(fs::symlink_metadata(scenario.target.join("dir1/sub1/file2")).is_err());
    assert!(record_of(&scenario.env, &scenario.target.join("dir1/sub1/file2")).is_none());
}

#[test]
fn replacing_parent_obstruction_creates_dirs_and_deploys() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::create_dir_all(target.join("dir1")).unwrap();
        fs::write(target.join("dir1/sub1"), b"content1").unwrap();
        fs::write(source.join("file1"), b"content2").unwrap();
        r#"
[portal]
"file1" = "dir1/sub1/file2"
"#
    });

    let prompter = Prompt::once(ObstructionChoice::Replace);
    let status = scenario
        .try_run_with_prompter(&prompter)
        .expect("apply failed");

    assert_eq!(status, ExitStatus::Success);
    assert_eq!(prompter.calls(), 1);
    assert!(
        fs::metadata(scenario.target.join("dir1/sub1"))
            .unwrap()
            .is_dir()
    );
    assert_eq!(
        fs::read(scenario.target.join("dir1/sub1/file2")).unwrap(),
        b"content2"
    );
    assert!(record_of(&scenario.env, &scenario.target.join("dir1/sub1/file2")).is_some());
}

#[test]
fn directory_symlink_parent_traversed_without_prompt() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::create_dir_all(target.join("dir1")).unwrap();
        symlink("dir1", target.join("link1")).unwrap();
        fs::write(source.join("file1"), b"content1").unwrap();
        r#"
[portal]
"file1" = "link1/file2"
"#
    });

    scenario.run();

    assert_eq!(
        fs::read(scenario.target.join("dir1/file2")).unwrap(),
        b"content1"
    );
    assert_eq!(
        fs::read(scenario.target.join("link1/file2")).unwrap(),
        b"content1"
    );
    assert!(record_of(&scenario.env, &scenario.target.join("link1/file2")).is_some());
}

#[test]
fn dangling_parent_symlink_replaced_leaving_external_safe() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::write(target.parent().unwrap().join("file1"), b"content1").unwrap();
        symlink("../file1", target.join("link1")).unwrap();
        fs::write(source.join("file2"), b"content2").unwrap();
        r#"
[portal]
"file2" = "link1/file3"
"#
    });

    let prompter = Prompt::once(ObstructionChoice::Replace);
    let status = scenario
        .try_run_with_prompter(&prompter)
        .expect("apply failed");

    assert_eq!(status, ExitStatus::Success);
    assert_eq!(prompter.calls(), 1);
    assert_eq!(
        fs::read(scenario.target.parent().unwrap().join("file1")).unwrap(),
        b"content1"
    );
    assert!(
        fs::metadata(scenario.target.join("link1"))
            .unwrap()
            .is_dir()
    );
    assert_eq!(
        fs::read(scenario.target.join("link1/file3")).unwrap(),
        b"content2"
    );
    assert!(record_of(&scenario.env, &scenario.target.join("link1/file3")).is_some());
}
