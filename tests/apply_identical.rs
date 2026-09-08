mod common;

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use common::{ApplyScenario, prompt_count};
use dotrift::ExitStatus;
use dotrift::commands::apply::{ApplyOptions, ObstructionChoice, test_hooks::set_prompt_choice};
use dotrift::hash::hash_bytes;
use dotrift::state::{Kind, StateDatabase};
use test_case::test_case;

const REPLACE_IDENTICAL: &str = "[apply]\nreplace-identical = true\n";

fn record_of(path: &Path) -> Option<dotrift::state::StateRecord> {
    StateDatabase::open()
        .expect("cannot open state database")
        .record(path)
        .unwrap()
}

fn symlink_setup(identical: bool) -> impl Fn(&Path, &Path) -> &'static str {
    move |source: &Path, target: &Path| {
        fs::write(source.join("file.txt"), b"new").unwrap();
        let link_target = if identical {
            source.join("file.txt")
        } else {
            target.join("elsewhere")
        };
        symlink(&link_target, target.join("target.txt")).unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    }
}

#[test_case(
    symlink_setup(true),
    None,
    ExitStatus::Success,
    |source: &Path, target: &Path| {
        assert_eq!(
            fs::read_link(target.join("target.txt")).unwrap(),
            source.join("file.txt")
        );
        let record = record_of(&target.join("target.txt")).unwrap();
        assert_eq!(record.kind, Kind::Symlink);
        assert_eq!(record.source_path, source.join("file.txt"));
        assert_eq!(prompt_count(), 0);
    }
    ; "identical_symlink_obstruction_replaced_without_prompt"
)]
fn symlink_obstruction_behaviors(
    setup: impl Fn(&Path, &Path) -> &'static str,
    choice: Option<ObstructionChoice>,
    expected_status: ExitStatus,
    assert: impl Fn(&Path, &Path),
) {
    let scenario = ApplyScenario::new(setup);
    scenario.env.write_global_config(REPLACE_IDENTICAL);
    if let Some(choice) = choice {
        set_prompt_choice(choice);
    }

    let status = scenario.try_run().expect("apply failed");

    assert_eq!(status, expected_status);
    assert(&scenario.source, &scenario.target);
}

#[test_case(
    |source: &Path, target: &Path| {
        fs::write(source.join("file.txt"), b"same").unwrap();
        fs::write(target.join("target.txt"), b"same").unwrap();
        fs::set_permissions(
            target.join("target.txt"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"copy\", mode = \"644\" }\n"
    },
    None,
    ExitStatus::Success,
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"same");
        let mode = fs::metadata(target.join("target.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o644);
        let record = record_of(&target.join("target.txt")).unwrap();
        assert_eq!(record.kind, Kind::File);
        assert_eq!(record.content_hash, Some(hash_bytes(b"same")));
        assert_eq!(prompt_count(), 0);
    }
    ; "identical_bytes_with_different_mode_replaced_and_rule_mode_reapplied"
)]
#[test_case(
    |source: &Path, target: &Path| {
        fs::write(source.join("file.txt"), b"new").unwrap();
        fs::write(target.join("target.txt"), b"old").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"copy\" }\n"
    },
    Some(ObstructionChoice::Skip),
    ExitStatus::Skipped,
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"old");
        assert_eq!(prompt_count(), 1);
    }
    ; "divergent_copy_obstruction_still_prompts"
)]
fn copy_obstruction_behaviors(
    setup: impl Fn(&Path, &Path) -> &'static str,
    choice: Option<ObstructionChoice>,
    expected_status: ExitStatus,
    assert: impl Fn(&Path, &Path),
) {
    let scenario = ApplyScenario::new(setup);
    scenario.env.write_global_config(REPLACE_IDENTICAL);
    if let Some(choice) = choice {
        set_prompt_choice(choice);
    }

    let status = scenario.try_run().expect("apply failed");

    assert_eq!(status, expected_status);
    assert(&scenario.source, &scenario.target);
}

fn template_setup(target_bytes: &[u8]) -> impl Fn(&Path, &Path) -> &'static str {
    move |source: &Path, target: &Path| {
        fs::write(
            source.join("dotrift_data.toml"),
            "[variable]\nmessage = \"hello\"\n",
        )
        .unwrap();
        fs::write(source.join("greeting.txt"), "{{ message }}\n").unwrap();
        fs::write(target.join("target.txt"), target_bytes).unwrap();
        "[portal]\n\"greeting.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"template\" }\n"
    }
}

#[test_case(
    template_setup(b"hello\n"),
    None,
    ExitStatus::Success,
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"hello\n");
        assert_eq!(prompt_count(), 0);
    }
    ; "identical_template_obstruction_replaced_without_prompt"
)]
fn template_obstruction_behaviors(
    setup: impl Fn(&Path, &Path) -> &'static str,
    choice: Option<ObstructionChoice>,
    expected_status: ExitStatus,
    assert: impl Fn(&Path, &Path),
) {
    let scenario = ApplyScenario::new(setup);
    scenario.env.write_global_config(REPLACE_IDENTICAL);
    if let Some(choice) = choice {
        set_prompt_choice(choice);
    }

    let status = scenario.try_run().expect("apply failed");

    assert_eq!(status, expected_status);
    assert(&scenario.source, &scenario.target);
}

#[test]
fn parent_obstruction_still_prompts() {
    let scenario = ApplyScenario::new(|source: &Path, target: &Path| {
        fs::write(source.join("file.txt"), b"new").unwrap();
        fs::write(target.join("a"), b"occupied").unwrap();
        "[portal]\n\"file.txt\" = \"a/b.txt\"\n"
    });
    scenario.env.write_global_config(REPLACE_IDENTICAL);
    set_prompt_choice(ObstructionChoice::Replace);

    let status = scenario.try_run().expect("apply failed");

    assert_eq!(status, ExitStatus::Success);
    assert_eq!(fs::read(scenario.target.join("a/b.txt")).unwrap(), b"new");
    assert_eq!(prompt_count(), 1);
}

#[test]
fn replace_all_latch_subsumes_the_identical_check() {
    let scenario = ApplyScenario::new(|source: &Path, target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        fs::write(target.join("a.txt"), b"divergent").unwrap();
        fs::write(target.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"b.txt\"\n"
    });
    scenario.env.write_global_config(REPLACE_IDENTICAL);
    set_prompt_choice(ObstructionChoice::ReplaceAll);

    let status = scenario.try_run().expect("apply failed");

    assert_eq!(status, ExitStatus::Success);
    assert_eq!(fs::read(scenario.target.join("a.txt")).unwrap(), b"A");
    assert_eq!(fs::read(scenario.target.join("b.txt")).unwrap(), b"B");
    assert_eq!(prompt_count(), 1);
}

fn dry_run_line(scenario: &ApplyScenario, path: &Path) -> String {
    dotrift::report::clear();
    scenario
        .try_run_with_options(ApplyOptions {
            dry_run: true,
            ..Default::default()
        })
        .expect("apply failed");
    let output = dotrift::report::take_output();
    output
        .lines()
        .find(|line| line.contains(path.to_str().unwrap()))
        .unwrap_or_else(|| panic!("no dry-run line for `{}` in:\n{output}", path.display()))
        .to_string()
}

#[test]
fn dry_run_reports_identical_obstruction_as_replaced() {
    let scenario = ApplyScenario::new(|source: &Path, target: &Path| {
        fs::write(source.join("file.txt"), b"same").unwrap();
        fs::write(target.join("target.txt"), b"same").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"copy\" }\n"
    });
    scenario.env.write_global_config(REPLACE_IDENTICAL);

    let line = dry_run_line(&scenario, &scenario.target.join("target.txt"));

    assert!(line.starts_with("replaced "), "{line}");
    assert!(line.contains("[copy]"), "{line}");
    assert_eq!(
        fs::read(scenario.target.join("target.txt")).unwrap(),
        b"same"
    );
    assert_eq!(prompt_count(), 0);
}

#[test]
fn dry_run_reports_template_obstruction_as_obstruction_without_rendering() {
    let scenario = ApplyScenario::new(|source: &Path, target: &Path| {
        fs::write(
            source.join("dotrift_data.toml"),
            "[variable]\nmessage = \"hello\"\n",
        )
        .unwrap();
        fs::write(source.join("greeting.txt"), "{{ message }}\n").unwrap();
        fs::write(target.join("target.txt"), b"hello\n").unwrap();
        "[portal]\n\"greeting.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"template\" }\n"
    });
    scenario.env.write_global_config(REPLACE_IDENTICAL);

    let line = dry_run_line(&scenario, &scenario.target.join("target.txt"));

    assert!(line.starts_with("obstruction "), "{line}");
}
