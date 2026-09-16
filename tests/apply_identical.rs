mod common;

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use common::{ApplyScenario, Prompt, record_of};
use dotrift::commands::apply::ApplyOptions;
use dotrift::deploy::ObstructionChoice;
use dotrift::state::Kind;
use dotrift::state::hash_bytes;

const REPLACE_IDENTICAL: &str = "[apply]\nreplace-identical = true\n";

#[test]
fn identical_symlink_obstruction_is_replaced_silently() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), b"content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });
    scenario.env.write_global_config(REPLACE_IDENTICAL);
    let entry = scenario.target.join("file1");
    symlink(scenario.source.join("file1"), &entry).unwrap();
    let prompter = Prompt::never();

    let status = scenario
        .try_run_with_prompter(&prompter)
        .expect("apply failed");

    assert_eq!(status, dotrift::ExitStatus::Success);
    assert_eq!(prompter.calls(), 0);
    assert_eq!(
        fs::read_link(&entry).unwrap(),
        scenario.source.join("file1")
    );
    let record = record_of(&scenario.env, &entry).expect("record missing");
    assert_eq!(record.kind, Kind::Symlink);
    assert_eq!(record.source_path, scenario.source.join("file1"));
}

#[test]
fn identical_bytes_with_different_mode_are_replaced_and_rule_mode_reapplied() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::write(source.join("file1"), b"content1").unwrap();
        fs::write(target.join("file1"), b"content1").unwrap();
        fs::set_permissions(target.join("file1"), std::fs::Permissions::from_mode(0o600)).unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "copy", mode = "644" }
"#
    });
    scenario.env.write_global_config(REPLACE_IDENTICAL);
    let prompter = Prompt::never();

    let status = scenario
        .try_run_with_prompter(&prompter)
        .expect("apply failed");

    assert_eq!(status, dotrift::ExitStatus::Success);
    assert_eq!(prompter.calls(), 0);
    let entry = scenario.target.join("file1");
    assert_eq!(fs::read(&entry).unwrap(), b"content1");
    let mode = fs::metadata(&entry).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o644);
    let record = record_of(&scenario.env, &entry).expect("record missing");
    assert_eq!(record.kind, Kind::File);
    assert_eq!(
        record.content_hash.as_deref(),
        Some(hash_bytes(b"content1").as_str())
    );
}

#[test]
fn divergent_copy_obstruction_still_prompts() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::write(source.join("file1"), b"content2").unwrap();
        fs::write(target.join("file1"), b"content1").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "copy" }
"#
    });
    scenario.env.write_global_config(REPLACE_IDENTICAL);
    let prompter = Prompt::once(ObstructionChoice::Skip);

    let status = scenario
        .try_run_with_prompter(&prompter)
        .expect("apply failed");

    assert_eq!(prompter.calls(), 1);
    assert_eq!(status, dotrift::ExitStatus::Skipped);
    assert_eq!(
        fs::read(scenario.target.join("file1")).unwrap(),
        b"content1"
    );
}

#[test]
fn identical_template_obstruction_is_replaced_silently() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::write(
            source.join("dotrift_data.toml"),
            "[variable]\nstr = \"str\"\n",
        )
        .unwrap();
        fs::write(source.join("file1"), "{{ str }}\n").unwrap();
        fs::write(target.join("file1"), b"str\n").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "template" }
"#
    });
    scenario.env.write_global_config(REPLACE_IDENTICAL);
    let prompter = Prompt::never();

    let status = scenario
        .try_run_with_prompter(&prompter)
        .expect("apply failed");

    assert_eq!(status, dotrift::ExitStatus::Success);
    assert_eq!(prompter.calls(), 0);
    assert_eq!(fs::read(scenario.target.join("file1")).unwrap(), b"str\n");
}

#[test]
fn parent_obstruction_still_prompts() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::write(source.join("file1"), b"content1").unwrap();
        fs::write(target.join("dir1"), b"content1").unwrap();
        r#"
[portal]
"file1" = "dir1/sub1/file1"
"#
    });
    scenario.env.write_global_config(REPLACE_IDENTICAL);
    let prompter = Prompt::once(ObstructionChoice::Replace);

    let status = scenario
        .try_run_with_prompter(&prompter)
        .expect("apply failed");

    assert_eq!(prompter.calls(), 1);
    assert_eq!(status, dotrift::ExitStatus::Success);
    assert_eq!(
        fs::read(scenario.target.join("dir1/sub1/file1")).unwrap(),
        b"content1"
    );
}

#[test]
fn replace_all_latch_subsumes_the_identical_check() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::write(source.join("file1"), b"content1").unwrap();
        fs::write(source.join("file2"), b"content2").unwrap();
        fs::write(target.join("file1"), b"other").unwrap();
        fs::write(target.join("file2"), b"content2").unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "file2"
"#
    });
    scenario.env.write_global_config(REPLACE_IDENTICAL);
    let prompter = Prompt::once(ObstructionChoice::ReplaceAll);

    let status = scenario
        .try_run_with_prompter(&prompter)
        .expect("apply failed");

    assert_eq!(prompter.calls(), 1);
    assert_eq!(status, dotrift::ExitStatus::Success);
    assert_eq!(
        fs::read(scenario.target.join("file1")).unwrap(),
        b"content1"
    );
    assert_eq!(
        fs::read(scenario.target.join("file2")).unwrap(),
        b"content2"
    );
}

fn dry_run_line(scenario: &ApplyScenario, path: &Path) -> String {
    dotrift::report::clear();
    dotrift::commands::apply::run_with_options(
        &scenario.source,
        Some(scenario.target.clone()),
        ApplyOptions {
            dry_run: true,
            ..Default::default()
        },
        scenario.env.env(),
        false,
    )
    .expect("apply failed");
    let output = dotrift::report::take_output();
    output
        .lines()
        .find(|line| line.contains(path.to_str().unwrap()))
        .unwrap_or_else(|| panic!("no dry-run line for `{}` in:\n{output}", path.display()))
        .to_string()
}

#[test]
fn dry_run_reports_identical_obstruction_as_replaced_without_changing_it() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::write(source.join("file1"), b"content1").unwrap();
        fs::write(target.join("file1"), b"content1").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "copy" }
"#
    });
    scenario.env.write_global_config(REPLACE_IDENTICAL);

    let line = dry_run_line(&scenario, &scenario.target.join("file1"));

    assert!(line.starts_with("replaced "), "{line}");
    assert!(line.contains("[copy]"), "{line}");
    assert_eq!(
        fs::read(scenario.target.join("file1")).unwrap(),
        b"content1"
    );
}

#[test]
fn dry_run_reports_template_obstruction_without_rendering_it() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::write(
            source.join("dotrift_data.toml"),
            "[variable]\nstr = \"str\"\n",
        )
        .unwrap();
        fs::write(source.join("file1"), "{{ str }}\n").unwrap();
        fs::write(target.join("file1"), b"str\n").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "template" }
"#
    });
    scenario.env.write_global_config(REPLACE_IDENTICAL);

    let line = dry_run_line(&scenario, &scenario.target.join("file1"));

    assert!(line.starts_with("obstruction "), "{line}");
}
