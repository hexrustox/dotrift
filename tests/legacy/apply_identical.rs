mod common;

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use common::{ApplyScenario, Prompt, record_of};
use dotrift::ExitStatus;
use dotrift::commands::apply::ApplyOptions;
use dotrift::deploy::ObstructionChoice;
use dotrift::state::Kind;
use dotrift::state::hash_bytes;
use test_case::test_case;

const REPLACE_IDENTICAL: &str = "[apply]\nreplace-identical = true\n";

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
    |ApplyScenario { source, target, env, .. }: &ApplyScenario| {
        assert_eq!(
            fs::read_link(target.join("target.txt")).unwrap(),
            source.join("file.txt")
        );
        let record = record_of(env, &target.join("target.txt")).unwrap();
        assert_eq!(record.kind, Kind::Symlink);
        assert_eq!(record.source_path, source.join("file.txt"));
    }
    ; "identical_symlink_obstruction_replaced_without_prompt"
)]
fn symlink_obstruction_behaviors(
    setup: impl Fn(&Path, &Path) -> &'static str,
    choice: Option<ObstructionChoice>,
    expected_status: ExitStatus,
    assert: impl Fn(&ApplyScenario),
) {
    let scenario = ApplyScenario::new(setup);
    scenario.env.write_global_config(REPLACE_IDENTICAL);
    let status = match choice {
        Some(choice) => {
            let prompter = Prompt::once(choice);
            let status = scenario
                .try_run_with_prompter(&prompter)
                .expect("apply failed");
            assert_eq!(prompter.calls(), 1);
            status
        }
        // Identical obstructions replace without prompting; the panicking
        // default proves no prompt fires.
        None => scenario.try_run().expect("apply failed"),
    };

    assert_eq!(status, expected_status);
    assert(&scenario);
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
    |ApplyScenario { target, env, .. }: &ApplyScenario| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"same");
        let mode = fs::metadata(target.join("target.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o644);
        let record = record_of(env, &target.join("target.txt")).unwrap();
        assert_eq!(record.kind, Kind::File);
        assert_eq!(record.content_hash, Some(String::from(hash_bytes(b"same"))));
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
    |ApplyScenario { target, .. }: &ApplyScenario| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"old");
    }
    ; "divergent_copy_obstruction_still_prompts"
)]
fn copy_obstruction_behaviors(
    setup: impl Fn(&Path, &Path) -> &'static str,
    choice: Option<ObstructionChoice>,
    expected_status: ExitStatus,
    assert: impl Fn(&ApplyScenario),
) {
    let scenario = ApplyScenario::new(setup);
    scenario.env.write_global_config(REPLACE_IDENTICAL);
    let status = match choice {
        Some(choice) => {
            let prompter = Prompt::once(choice);
            let status = scenario
                .try_run_with_prompter(&prompter)
                .expect("apply failed");
            assert_eq!(prompter.calls(), 1);
            status
        }
        // Identical obstructions replace without prompting; the panicking
        // default proves no prompt fires.
        None => scenario.try_run().expect("apply failed"),
    };

    assert_eq!(status, expected_status);
    assert(&scenario);
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
    |ApplyScenario { target, .. }: &ApplyScenario| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"hello\n");
    }
    ; "identical_template_obstruction_replaced_without_prompt"
)]
fn template_obstruction_behaviors(
    setup: impl Fn(&Path, &Path) -> &'static str,
    choice: Option<ObstructionChoice>,
    expected_status: ExitStatus,
    assert: impl Fn(&ApplyScenario),
) {
    let scenario = ApplyScenario::new(setup);
    scenario.env.write_global_config(REPLACE_IDENTICAL);
    let status = match choice {
        Some(choice) => {
            let prompter = Prompt::once(choice);
            let status = scenario
                .try_run_with_prompter(&prompter)
                .expect("apply failed");
            assert_eq!(prompter.calls(), 1);
            status
        }
        // Identical obstructions replace without prompting; the panicking
        // default proves no prompt fires.
        None => scenario.try_run().expect("apply failed"),
    };

    assert_eq!(status, expected_status);
    assert(&scenario);
}

#[test]
fn parent_obstruction_still_prompts() {
    let scenario = ApplyScenario::new(|source: &Path, target: &Path| {
        fs::write(source.join("file.txt"), b"new").unwrap();
        fs::write(target.join("a"), b"occupied").unwrap();
        "[portal]\n\"file.txt\" = \"a/b.txt\"\n"
    });
    scenario.env.write_global_config(REPLACE_IDENTICAL);
    let prompter = Prompt::once(ObstructionChoice::Replace);

    let status = scenario
        .try_run_with_prompter(&prompter)
        .expect("apply failed");

    assert_eq!(prompter.calls(), 1);
    assert_eq!(status, ExitStatus::Success);
    assert_eq!(fs::read(scenario.target.join("a/b.txt")).unwrap(), b"new");
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
    let prompter = Prompt::once(ObstructionChoice::ReplaceAll);

    let status = scenario
        .try_run_with_prompter(&prompter)
        .expect("apply failed");

    assert_eq!(prompter.calls(), 1);
    assert_eq!(status, ExitStatus::Success);
    assert_eq!(fs::read(scenario.target.join("a.txt")).unwrap(), b"A");
    assert_eq!(fs::read(scenario.target.join("b.txt")).unwrap(), b"B");
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
