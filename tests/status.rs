mod common;

use std::fs;

use common::{ApplyScenario, TestEnv, snapshot_settings, test_name};

fn status_output(env: &TestEnv, color: bool) -> String {
    dotrift::report::clear();
    dotrift::commands::status::run(env.env(), color).expect("status failed");
    dotrift::report::take_output()
}

/// Deploys three entries in non-sorted config order: a symlink (`file1`), a
/// copy (`file2`), and a nested symlink (`dir1/file1`).
fn mixed_scenario() -> ApplyScenario {
    ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content1").unwrap();
        fs::create_dir_all(source.join("dir1")).unwrap();
        fs::write(source.join("dir1/file1"), "content1").unwrap();
        r#"
[portal]
"file2" = "file2"
"file1" = "file1"
"dir1" = "dir1"

[rule]
"file2" = { type = "copy" }
"#
    })
}

fn strip_ansi(value: &str) -> String {
    let mut stripped = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(next) = chars.next() {
        if next == '\x1b' {
            for next in chars.by_ref() {
                if next == 'm' {
                    break;
                }
            }
        } else {
            stripped.push(next);
        }
    }
    stripped
}

#[test]
fn status_reports_nothing_without_database() {
    let env = TestEnv::new();

    let output = status_output(&env, false);

    assert_eq!(output, "");
    assert!(!env.path("state/state.sqlite").exists());
}

#[test]
fn status_treats_table_less_database_as_empty() {
    let env = TestEnv::new();
    fs::create_dir_all(env.path("state")).unwrap();
    // A valid, zero-byte SQLite file: parseable, but holding no tables.
    fs::write(env.path("state/state.sqlite"), b"").unwrap();

    let output = status_output(&env, false);

    assert_eq!(output, "");
}

#[test]
fn status_reports_nothing_for_empty_database() {
    let env = TestEnv::new();
    assert!(
        env.database().managed_paths().unwrap().is_empty(),
        "expected a fresh database with no records"
    );

    let output = status_output(&env, false);

    assert_eq!(output, "");
}

#[test]
fn status_prints_sorted_lines_with_verdicts() {
    let scenario = mixed_scenario();
    scenario.run();
    fs::write(scenario.target.join("file2"), "content2").unwrap();
    fs::remove_file(scenario.target.join("dir1/file1")).unwrap();

    let output = status_output(&scenario.env, false);

    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 3, "expected one line per record:\n{output}");
    assert!(
        lines[0].starts_with("unmanaged") && lines[0].contains("dir1/file1"),
        "expected the missing nested entry first:\n{output}"
    );
    assert!(
        lines[1].starts_with("managed") && lines[1].contains("target/file1 <-"),
        "expected the intact symlink second:\n{output}"
    );
    assert!(
        lines[2].starts_with("unmanaged") && lines[2].contains("target/file2 <-"),
        "expected the changed copy last:\n{output}"
    );

    snapshot_settings(&scenario.env).bind(|| {
        insta::assert_snapshot!(test_name(), output);
    });
}

#[test]
fn status_color_forced_layout_is_unchanged() {
    let scenario = mixed_scenario();
    scenario.run();
    fs::write(scenario.target.join("file2"), "content2").unwrap();
    fs::remove_file(scenario.target.join("dir1/file1")).unwrap();

    let plain = status_output(&scenario.env, false);
    let colored = status_output(&scenario.env, true);

    assert!(
        colored.contains('\x1b'),
        "expected forced color to emit escape sequences:\n{colored}"
    );
    assert_eq!(
        strip_ansi(&colored),
        plain,
        "color must not change the status layout"
    );

    snapshot_settings(&scenario.env).bind(|| {
        insta::assert_snapshot!(test_name(), colored);
    });
}
