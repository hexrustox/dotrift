mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::{ApplyScenario, EnvVarGuard, TestEnv, snapshot_settings, test_name};
use dotrift::commands::apply::{ObstructionChoice, test_hooks::set_prompt_choices};
use test_case::test_case;

fn capture_script(env: &TestEnv) -> (PathBuf, PathBuf) {
    let output = env.path("viewdiff.txt");
    let script = env.path("capture-pager.sh");
    fs::write(&script, format!("#!/bin/sh\ncat > {}\n", output.display())).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    (script, output)
}

fn copy_diff_setup(source: &Path, target: &Path) -> &'static str {
    fs::write(source.join("file.txt"), b"new content\n").unwrap();
    fs::write(target.join("target.txt"), b"old content\n").unwrap();
    "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"copy\" }\n"
}

enum PagerChoice {
    CaptureScript,
    MissingBinary,
    Unset,
}

fn resolve_pager(choice: PagerChoice, env: &TestEnv) -> (Option<String>, Option<PathBuf>) {
    match choice {
        PagerChoice::CaptureScript => {
            let (script, output) = capture_script(env);
            (Some(script.to_str().unwrap().to_owned()), Some(output))
        }
        PagerChoice::MissingBinary => (
            Some(env.path("no-such-pager").to_string_lossy().into_owned()),
            None,
        ),
        PagerChoice::Unset => (None, None),
    }
}

#[test_case(copy_diff_setup ; "shows_single_line_copy_diff")]
#[test_case(
    |source: &Path, target: &Path| {
        fs::write(source.join("dotrift_data.toml"), "[variable]\nmessage = \"hello\"\n").unwrap();
        fs::write(source.join("greeting.txt"), "{{ message }}\n").unwrap();
        fs::write(target.join("target.txt"), b"old\n").unwrap();
        "[portal]\n\"greeting.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"template\" }\n"
    }
    ; "shows_rendered_template_diff"
)]
fn view_diff_prompt_output(setup: impl Fn(&Path, &Path) -> &'static str) {
    let scenario = ApplyScenario::new(setup);
    let (script, output) = capture_script(&scenario.env);
    let _guard = EnvVarGuard::set([
        ("DOTRIFT_PAGER", Some(script.to_str().unwrap())),
        ("PAGER", Some("")),
    ]);
    set_prompt_choices([ObstructionChoice::ViewDiff, ObstructionChoice::Skip]);
    scenario.run();
    let diff = fs::read_to_string(output).unwrap();
    snapshot_settings(&scenario.env).bind(|| {
        insta::assert_snapshot!(test_name(), &diff);
    });
}

#[test_case(Some(""), PagerChoice::CaptureScript ; "blank_dotrift_pager_falls_back_to_pager")]
#[test_case(Some("   "), PagerChoice::CaptureScript ; "whitespace_dotrift_pager_falls_back_to_pager")]
#[test_case(None, PagerChoice::Unset ; "no_pager_configured_prints_diff_to_stdout")]
#[test_case(Some(""), PagerChoice::MissingBinary ; "failing_pager_falls_back_to_stdout")]
fn pager_fallback(dotrift_pager: Option<&str>, pager: PagerChoice) {
    let scenario = ApplyScenario::new(copy_diff_setup);
    let (pager_value, output) = resolve_pager(pager, &scenario.env);
    let _guard = EnvVarGuard::set([
        ("DOTRIFT_PAGER", dotrift_pager),
        ("PAGER", pager_value.as_deref()),
    ]);
    set_prompt_choices([ObstructionChoice::ViewDiff, ObstructionChoice::Skip]);
    dotrift::capture::clear();
    scenario.run();
    let diff = match output {
        Some(path) => fs::read_to_string(path).unwrap(),
        None => dotrift::capture::take(),
    };
    snapshot_settings(&scenario.env).bind(|| {
        insta::assert_snapshot!(test_name(), &diff);
    });
}

#[test]
fn failing_dotrift_pager_raises_error() {
    let scenario = ApplyScenario::new(copy_diff_setup);
    let (pager_value, _) = resolve_pager(PagerChoice::MissingBinary, &scenario.env);
    let _guard = EnvVarGuard::set([
        ("DOTRIFT_PAGER", pager_value.as_deref()),
        ("PAGER", Some("")),
    ]);
    set_prompt_choices([ObstructionChoice::ViewDiff, ObstructionChoice::Skip]);
    let error = scenario.try_run().unwrap_err();
    let rendered = format!("{error}");
    assert!(rendered.contains("cannot run DOTRIFT_PAGER"), "{rendered}");
}
