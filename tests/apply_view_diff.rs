mod common;

use std::fs;
use std::path::Path;

use common::{
    ApplyScenario, PagerChoice, Prompt, argv_capture_script, argv_diff_script, capture_script,
    capture_script_named, config_diff_toml, config_pager_toml, diff_script, resolve_pager,
    snapshot_settings, test_name,
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

fn argv_lines(output: &Path) -> Vec<String> {
    fs::read_to_string(output)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
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
    assert!(
        rendered.contains("cannot run `DOTRIFT_PAGER`"),
        "{rendered}"
    );
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

#[test]
fn configured_diff_command_receives_substituted_placeholders() {
    let scenario = ApplyScenario::new(copy_setup);
    let (script, output) = argv_diff_script(&scenario.env);
    scenario.env.write_global_config(&config_diff_toml(
        &script,
        &[
            "t:${target}",
            "s:${source}",
            "tl:${target-label}",
            "sl:${source-label}",
        ],
    ));
    let _guard = scenario
        .env
        .set_vars([("DOTRIFT_PAGER", None), ("PAGER", None)]);
    view_diff_and_skip(&scenario);

    let target = scenario.target.join("file2");
    let source = scenario.source.join("file1");
    assert_eq!(
        argv_lines(&output),
        [
            format!("t:{}", target.display()),
            format!("s:{}", source.display()),
            format!("tl:{}", target.display()),
            format!("sl:{}", source.display()),
        ]
    );
}

#[test]
fn configured_diff_command_appends_both_paths_when_args_name_neither() {
    let scenario = ApplyScenario::new(copy_setup);
    let (script, output) = argv_diff_script(&scenario.env);
    scenario
        .env
        .write_global_config(&config_diff_toml(&script, &["-u"]));
    let _guard = scenario
        .env
        .set_vars([("DOTRIFT_PAGER", None), ("PAGER", None)]);
    view_diff_and_skip(&scenario);

    let target = scenario.target.join("file2");
    let source = scenario.source.join("file1");
    assert_eq!(
        argv_lines(&output),
        [
            "-u".to_owned(),
            target.to_string_lossy().into_owned(),
            source.to_string_lossy().into_owned(),
        ]
    );
}

#[test]
fn configured_diff_command_keeps_embedded_form_as_one_argument() {
    let scenario = ApplyScenario::new(copy_setup);
    let (script, output) = argv_diff_script(&scenario.env);
    scenario
        .env
        .write_global_config(&config_diff_toml(&script, &["--pair=${target}:${source}"]));
    let _guard = scenario
        .env
        .set_vars([("DOTRIFT_PAGER", None), ("PAGER", None)]);
    view_diff_and_skip(&scenario);

    let target = scenario.target.join("file2");
    let source = scenario.source.join("file1");
    assert_eq!(
        argv_lines(&output),
        [format!("--pair={}:{}", target.display(), source.display())]
    );
}

#[test]
fn configured_diff_command_gets_raw_labels_and_rendered_source_for_a_template() {
    let scenario = ApplyScenario::new(template_setup);
    let script = diff_script(
        &scenario.env,
        "script1",
        "for arg in \"$@\"; do\n  printf '%s\\n' \"$arg\"\n  if [ -f \"$arg\" ]; then cat \"$arg\"; fi\ndone\n",
    );
    scenario.env.write_global_config(&config_diff_toml(
        &script,
        &[
            "${target-label}",
            "${source-label}",
            "${target}",
            "${source}",
        ],
    ));
    let (pager, pager_output) = capture_script(&scenario.env);
    let pager_value = script_string(&pager);
    let _guard = scenario.env.set_vars([
        ("DOTRIFT_PAGER", Some(pager_value.as_str())),
        ("PAGER", None),
    ]);
    view_diff_and_skip(&scenario);

    let paged = fs::read_to_string(&pager_output).unwrap();
    let target = scenario.target.join("file2");
    let source = scenario.source.join("file1");
    // Each path is followed by its content: the labels point at the raw
    // target and the template source, while `${source}` is the rendered
    // output.
    let rendered_path = paged.lines().nth(6).unwrap().to_owned();
    assert_eq!(
        paged.lines().collect::<Vec<_>>(),
        &[
            target.to_str().unwrap(),
            "content1",
            source.to_str().unwrap(),
            "{{ str }}",
            target.to_str().unwrap(),
            "content1",
            rendered_path.as_str(),
            "str",
        ][..],
        "{paged}"
    );
    assert_ne!(rendered_path, source.to_str().unwrap());
}

#[test]
fn configured_diff_command_flows_through_the_configured_pager() {
    let scenario = ApplyScenario::new(copy_setup);
    let (pager, pager_output) = capture_script(&scenario.env);
    let diff = diff_script(&scenario.env, "script2", "printf 'DIFF-OUTPUT\\n'\n");
    scenario.env.write_global_config(&format!(
        "{}\n{}",
        config_pager_toml(&pager, &[]),
        config_diff_toml(&diff, &[])
    ));
    let _guard = scenario
        .env
        .set_vars([("DOTRIFT_PAGER", None), ("PAGER", None)]);
    view_diff_and_skip(&scenario);

    let paged = fs::read_to_string(&pager_output).unwrap();
    assert_eq!(paged, "DIFF-OUTPUT\n", "{paged}");
}

#[test]
fn empty_configured_diff_command_uses_the_builtin_diff() {
    let scenario = ApplyScenario::new(copy_setup);
    let (pager, pager_output) = capture_script(&scenario.env);
    let pager_value = script_string(&pager);
    let _guard = scenario.env.set_vars([
        ("DOTRIFT_PAGER", Some(pager_value.as_str())),
        ("PAGER", None),
    ]);
    scenario
        .env
        .write_global_config("[diff]\ncommand = ''\nargs = ['--bogus-flag']\n");
    view_diff_and_skip(&scenario);

    let paged = fs::read_to_string(&pager_output).unwrap();
    assert!(
        paged.contains("-content1") && paged.contains("+content2"),
        "{paged}"
    );
    assert!(!paged.contains("--bogus-flag"), "{paged}");
}

#[test]
fn failing_configured_diff_command_fails_the_run() {
    let scenario = ApplyScenario::new(copy_setup);
    let missing = scenario.env.path("file3");
    let (pager, pager_output) = capture_script(&scenario.env);
    scenario
        .env
        .write_global_config(&config_diff_toml(&missing, &[]));
    let pager_value = script_string(&pager);
    let _guard = scenario.env.set_vars([
        ("DOTRIFT_PAGER", Some(pager_value.as_str())),
        ("PAGER", None),
    ]);

    let prompter = Prompt::sequence(DIFF_PROMPTS);
    let error = scenario.try_run_with_prompter(&prompter).unwrap_err();

    let rendered = format!("{error}");
    assert!(
        rendered.contains("cannot run the configured diff"),
        "{rendered}"
    );
    assert!(!pager_output.exists());
}

#[test]
fn exit_2_from_configured_diff_command_fails_the_run() {
    let scenario = ApplyScenario::new(copy_setup);
    let script = diff_script(&scenario.env, "script3", "exit 2\n");
    scenario
        .env
        .write_global_config(&config_diff_toml(&script, &[]));
    let _guard = scenario
        .env
        .set_vars([("DOTRIFT_PAGER", None), ("PAGER", None)]);

    let prompter = Prompt::sequence(DIFF_PROMPTS);
    let error = scenario.try_run_with_prompter(&prompter).unwrap_err();

    let rendered = format!("{error}");
    assert!(rendered.contains("exited with status"), "{rendered}");
    assert!(rendered.contains("status 2"), "{rendered}");
}

#[test]
fn exit_3_from_configured_diff_command_fails_the_run() {
    let scenario = ApplyScenario::new(copy_setup);
    let script = diff_script(&scenario.env, "script3", "exit 3\n");
    scenario
        .env
        .write_global_config(&config_diff_toml(&script, &[]));
    let _guard = scenario
        .env
        .set_vars([("DOTRIFT_PAGER", None), ("PAGER", None)]);

    let prompter = Prompt::sequence(DIFF_PROMPTS);
    let error = scenario.try_run_with_prompter(&prompter).unwrap_err();

    let rendered = format!("{error}");
    assert!(rendered.contains("exited with status"), "{rendered}");
    assert!(rendered.contains("status 3"), "{rendered}");
}

#[test]
fn exit_1_from_configured_diff_command_is_normal() {
    let scenario = ApplyScenario::new(copy_setup);
    let script = diff_script(
        &scenario.env,
        "script4",
        "printf 'DIFF-OUTPUT\\n'\nexit 1\n",
    );
    scenario
        .env
        .write_global_config(&config_diff_toml(&script, &[]));
    let (pager, pager_output) = capture_script(&scenario.env);
    let pager_value = script_string(&pager);
    let _guard = scenario.env.set_vars([
        ("DOTRIFT_PAGER", Some(pager_value.as_str())),
        ("PAGER", None),
    ]);

    view_diff_and_skip(&scenario);

    let paged = fs::read_to_string(&pager_output).unwrap();
    assert_eq!(paged, "DIFF-OUTPUT\n", "{paged}");
}
