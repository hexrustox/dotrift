mod common;

use std::fs;
use std::path::Path;

use common::{
    ApplyScenario, PagerChoice, Prompt, argv_capture_script, capture_script, capture_script_named,
    config_pager_toml, resolve_pager, snapshot_settings, test_name,
};
use dotrift::deploy::ObstructionChoice;
use test_case::test_case;

fn copy_setup(source: &Path, target: &Path) -> &'static str {
    fs::write(source.join("file1"), b"content2\n").unwrap();
    fs::write(target.join("file2"), b"content1\n").unwrap();
    r#"
[portal]
"file1" = "file2"

[rule]
"file2" = { type = "copy" }
"#
}

fn template_setup(source: &Path, target: &Path) -> &'static str {
    fs::write(
        source.join("dotrift_data.toml"),
        "[variable]\nstr = \"str\"\n",
    )
    .unwrap();
    fs::write(source.join("file1"), "{{ str }}\n").unwrap();
    fs::write(target.join("file2"), b"content1\n").unwrap();
    r#"
[portal]
"file1" = "file2"

[rule]
"file2" = { type = "template" }
"#
}

const DIFF_PROMPTS: [ObstructionChoice; 2] = [ObstructionChoice::ViewDiff, ObstructionChoice::Skip];

fn script_string(path: &Path) -> String {
    path.to_str().unwrap().to_owned()
}

fn view_diff_and_skip(scenario: &ApplyScenario) {
    let prompter = Prompt::sequence(DIFF_PROMPTS);
    scenario.run_with_prompter(&prompter);
}

#[test_case(copy_setup ; "copy_diff_shows_removed_and_added_lines")]
#[test_case(template_setup ; "template_diff_shows_rendered_source")]
fn diff_captured_through_dotrift_pager(setup: impl Fn(&Path, &Path) -> &'static str) {
    let scenario = ApplyScenario::new(setup);
    let (script, output) = capture_script(&scenario.env);
    let script_value = script_string(&script);
    let _guard = scenario.env.set_vars([
        ("DOTRIFT_PAGER", Some(script_value.as_str())),
        ("PAGER", Some("")),
    ]);
    view_diff_and_skip(&scenario);

    let diff = fs::read_to_string(output).unwrap();
    snapshot_settings(&scenario.env).bind(|| {
        insta::assert_snapshot!(test_name(), diff);
    });
}

#[test_case(Some(""), PagerChoice::CaptureScript ; "blank_dotrift_pager_falls_back_to_pager")]
#[test_case(Some("   "), PagerChoice::CaptureScript ; "whitespace_dotrift_pager_falls_back_to_pager")]
#[test_case(None, PagerChoice::Unset ; "no_pager_configured_prints_diff_to_stdout")]
#[test_case(Some(""), PagerChoice::MissingBinary ; "failing_pager_falls_back_to_stdout")]
fn pager_fallback(dotrift_pager: Option<&str>, pager: PagerChoice) {
    let scenario = ApplyScenario::new(copy_setup);
    let (pager_value, output) = resolve_pager(pager, &scenario.env);
    let _guard = scenario.env.set_vars([
        ("DOTRIFT_PAGER", dotrift_pager),
        ("PAGER", pager_value.as_deref()),
    ]);
    dotrift::report::clear();
    view_diff_and_skip(&scenario);

    let diff = match output {
        Some(path) => fs::read_to_string(path).unwrap(),
        None => dotrift::report::take_output(),
    };
    snapshot_settings(&scenario.env).bind(|| {
        insta::assert_snapshot!(test_name(), diff);
    });
}

#[test]
fn failing_dotrift_pager_raises_error() {
    let scenario = ApplyScenario::new(copy_setup);
    let (missing, _) = resolve_pager(PagerChoice::MissingBinary, &scenario.env);
    let _guard = scenario
        .env
        .set_vars([("DOTRIFT_PAGER", missing.as_deref()), ("PAGER", Some(""))]);

    let prompter = Prompt::sequence(DIFF_PROMPTS);
    let error = scenario.try_run_with_prompter(&prompter).unwrap_err();

    let rendered = format!("{error}");
    assert!(rendered.contains("cannot run DOTRIFT_PAGER"), "{rendered}");
}

#[test]
fn configured_pager_used_when_dotrift_pager_unset() {
    let scenario = ApplyScenario::new(copy_setup);
    let (config_script, config_output) = argv_capture_script(&scenario.env);
    let (env_script, env_output) = capture_script(&scenario.env);
    scenario
        .env
        .write_global_config(&config_pager_toml(&config_script, &[]));
    let env_value = script_string(&env_script);
    let _guard = scenario.env.set_vars([("PAGER", Some(env_value.as_str()))]);
    view_diff_and_skip(&scenario);

    let diff = fs::read_to_string(&config_output).unwrap();
    assert!(
        diff.contains("-content1") && diff.contains("+content2"),
        "{diff}"
    );
    assert!(!env_output.exists());
}

#[test]
fn dotrift_pager_overrides_configured_pager() {
    let scenario = ApplyScenario::new(copy_setup);
    let (config_script, config_output) = capture_script_named(&scenario.env, "config-pager");
    let (env_script, env_output) = capture_script_named(&scenario.env, "env-pager");
    scenario
        .env
        .write_global_config(&config_pager_toml(&config_script, &[]));
    let env_value = script_string(&env_script);
    let _guard = scenario
        .env
        .set_vars([("DOTRIFT_PAGER", Some(env_value.as_str())), ("PAGER", None)]);
    view_diff_and_skip(&scenario);

    let diff = fs::read_to_string(&env_output).unwrap();
    assert!(
        diff.contains("-content1") && diff.contains("+content2"),
        "{diff}"
    );
    assert!(!config_output.exists());
}

#[test]
fn empty_configured_pager_falls_through_to_pager() {
    let scenario = ApplyScenario::new(copy_setup);
    let (env_script, env_output) = capture_script(&scenario.env);
    let config_output = scenario.env.path("unused-pager-out.txt");
    scenario.env.write_global_config("[pager]\ncommand = ''\n");
    let env_value = script_string(&env_script);
    let _guard = scenario.env.set_vars([("PAGER", Some(env_value.as_str()))]);
    view_diff_and_skip(&scenario);

    let diff = fs::read_to_string(&env_output).unwrap();
    assert!(
        diff.contains("-content1") && diff.contains("+content2"),
        "{diff}"
    );
    assert!(!config_output.exists());
}

#[test]
fn configured_pager_args_are_literal() {
    let scenario = ApplyScenario::new(copy_setup);
    let (script, output) = argv_capture_script(&scenario.env);
    scenario
        .env
        .write_global_config(&config_pager_toml(&script, &["one two", "three", "-R"]));
    let _guard = scenario.env.set_vars([("PAGER", None)]);
    view_diff_and_skip(&scenario);

    let captured = fs::read_to_string(&output).unwrap();
    let lines = captured.lines().collect::<Vec<_>>();
    assert_eq!(&lines[..3], &["one two", "three", "-R"], "{captured}");
    assert!(captured.contains("-content1") && captured.contains("+content2"));
}

#[test]
fn failing_configured_pager_fails_the_run_without_falling_back() {
    let scenario = ApplyScenario::new(copy_setup);
    let missing = scenario.env.path("no-such-pager");
    let (env_script, env_output) = capture_script(&scenario.env);
    scenario
        .env
        .write_global_config(&config_pager_toml(&missing, &[]));
    let env_value = script_string(&env_script);
    let _guard = scenario.env.set_vars([("PAGER", Some(env_value.as_str()))]);

    let prompter = Prompt::sequence(DIFF_PROMPTS);
    let error = scenario.try_run_with_prompter(&prompter).unwrap_err();

    let rendered = format!("{error}");
    assert!(
        rendered.contains("cannot run the configured pager"),
        "{rendered}"
    );
    assert!(!env_output.exists());
}
