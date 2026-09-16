mod common;

use std::fs;
use std::os::unix::fs::symlink;

use common::{ApplyScenario, assert_error_chain, record_of};

#[test]
fn empty_variable_key_fails_before_deploying_anything() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });
    scenario.env.write_data_file("[variable]\n\"\" = \"str\"\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "empty key in `[variable]`");
    assert!(
        fs::symlink_metadata(scenario.target.join("file1")).is_err(),
        "nothing must be deployed when the data file fails to parse"
    );
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn missing_literal_portal_source_fails_before_deploying_anything() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "file2"
"#
    });

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "literal portal source");
    assert!(fs::symlink_metadata(scenario.target.join("file1")).is_err());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn template_render_failure_mid_run_preserves_completed_entries() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "{{ missing }}").unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "file2"

[rule]
"file1" = { type = "copy" }
"file2" = { type = "template" }
"#
    });

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "undefined variable");
    assert_eq!(
        fs::read(scenario.target.join("file1")).unwrap(),
        b"content1"
    );
    assert!(
        record_of(&scenario.env, &scenario.target.join("file1")).is_some(),
        "the entry completed before the failure must keep its record"
    );
    assert!(fs::symlink_metadata(scenario.target.join("file2")).is_err());
    assert!(record_of(&scenario.env, &scenario.target.join("file2")).is_none());
}

#[test]
fn deleted_source_between_runs_fails_preflight() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });

    scenario.run();
    fs::remove_file(scenario.source.join("file1")).unwrap();

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "literal portal source");
    assert!(
        fs::symlink_metadata(scenario.target.join("file1"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "the already deployed target must survive the failed preflight"
    );
}

#[test]
fn dangling_symlink_target_root_is_rejected_before_deployment() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });
    fs::remove_dir(&scenario.target).unwrap();
    symlink("file1", &scenario.target).unwrap();

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "is not a directory");
    assert!(
        fs::symlink_metadata(&scenario.target)
            .unwrap()
            .file_type()
            .is_symlink(),
        "the dangling target root must be left untouched"
    );
    assert!(fs::symlink_metadata(scenario.target.join("file1")).is_err());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn dangling_symlink_target_root_rejected_even_for_empty_deployment() {
    let scenario = ApplyScenario::new(|_source, _target| "");
    fs::remove_dir(&scenario.target).unwrap();
    symlink("file1", &scenario.target).unwrap();

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "is not a directory");
    assert!(
        fs::symlink_metadata(&scenario.target)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}
