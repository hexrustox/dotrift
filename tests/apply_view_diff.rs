mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::{ApplyScenario, EnvVarGuard, QueuePrompter, TestEnv, snapshot_settings, test_name};
use dotrift::deploy::ObstructionChoice;
use test_case::test_case;

fn capture_script_named(env: &TestEnv, name: &str) -> (PathBuf, PathBuf) {
    let output = env.path(format!("{name}.txt"));
    let script = env.path(format!("capture-{name}.sh"));
    fs::write(&script, format!("#!/bin/sh\ncat > {}\n", output.display())).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    (script, output)
}

fn capture_script(env: &TestEnv) -> (PathBuf, PathBuf) {
    capture_script_named(env, "pager")
}

/// A pager capture script that also records its arguments, one per line,
/// before the diff.
fn argv_capture_script(env: &TestEnv) -> (PathBuf, PathBuf) {
    let output = env.path("pager-argv.txt");
    let script = env.path("capture-argv-pager.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nfor arg in \"$@\"; do printf '%s\\n' \"$arg\" >> {}; done\ncat >> {}\n",
            output.display(),
            output.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    (script, output)
}

fn config_pager_toml(command: &Path, args: &[&str]) -> String {
    let args = args
        .iter()
        .map(|arg| format!("'{arg}'"))
        .collect::<Vec<_>>()
        .join(", ");
    if args.is_empty() {
        format!("[pager]\ncommand = '{}'\n", command.display())
    } else {
        format!(
            "[pager]\ncommand = '{}'\nargs = [{args}]\n",
            command.display()
        )
    }
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
    let prompter = QueuePrompter::sequence([ObstructionChoice::ViewDiff, ObstructionChoice::Skip]);
    scenario.run_with_prompter(&prompter);
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
    let prompter = QueuePrompter::sequence([ObstructionChoice::ViewDiff, ObstructionChoice::Skip]);
    dotrift::report::clear();
    scenario.run_with_prompter(&prompter);
    let diff = match output {
        Some(path) => fs::read_to_string(path).unwrap(),
        None => dotrift::report::take_output(),
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
    let prompter = QueuePrompter::sequence([ObstructionChoice::ViewDiff, ObstructionChoice::Skip]);
    let error = scenario.try_run_with_prompter(&prompter).unwrap_err();
    let rendered = format!("{error}");
    assert!(rendered.contains("cannot run DOTRIFT_PAGER"), "{rendered}");
}

#[test]
fn config_pager_used_when_dotrift_pager_unset() {
    let scenario = ApplyScenario::new(copy_diff_setup);
    let (config_script, config_output) = argv_capture_script(&scenario.env);
    let (env_pager_script, env_pager_output) = capture_script(&scenario.env);
    scenario
        .env
        .write_global_config(&config_pager_toml(&config_script, &[]));
    let _guard = EnvVarGuard::set([("PAGER", Some(env_pager_script.to_str().unwrap()))]);
    let prompter = QueuePrompter::sequence([ObstructionChoice::ViewDiff, ObstructionChoice::Skip]);

    scenario.run_with_prompter(&prompter);

    let diff = fs::read_to_string(&config_output).unwrap();
    assert!(
        diff.contains("-old content") && diff.contains("+new content"),
        "{diff}"
    );
    assert!(!env_pager_output.exists());
}

#[test]
fn dotrift_pager_overrides_config_pager() {
    let scenario = ApplyScenario::new(copy_diff_setup);
    let (config_script, config_output) = capture_script_named(&scenario.env, "config-pager");
    let (env_pager_script, env_pager_output) = capture_script_named(&scenario.env, "env-pager");
    scenario
        .env
        .write_global_config(&config_pager_toml(&config_script, &[]));
    let _guard = EnvVarGuard::set([
        ("DOTRIFT_PAGER", Some(env_pager_script.to_str().unwrap())),
        ("PAGER", None),
    ]);
    let prompter = QueuePrompter::sequence([ObstructionChoice::ViewDiff, ObstructionChoice::Skip]);

    scenario.run_with_prompter(&prompter);

    let diff = fs::read_to_string(&env_pager_output).unwrap();
    assert!(
        diff.contains("-old content") && diff.contains("+new content"),
        "{diff}"
    );
    assert!(!config_output.exists());
}

#[test]
fn empty_config_command_falls_through_to_pager() {
    let scenario = ApplyScenario::new(copy_diff_setup);
    let (env_pager_script, env_pager_output) = capture_script(&scenario.env);
    let config_output = scenario.env.path("unused-config-out.txt");
    scenario.env.write_global_config("[pager]\ncommand = ''\n");
    let _guard = EnvVarGuard::set([("PAGER", Some(env_pager_script.to_str().unwrap()))]);
    let prompter = QueuePrompter::sequence([ObstructionChoice::ViewDiff, ObstructionChoice::Skip]);

    scenario.run_with_prompter(&prompter);

    let diff = fs::read_to_string(&env_pager_output).unwrap();
    assert!(
        diff.contains("-old content") && diff.contains("+new content"),
        "{diff}"
    );
    assert!(!config_output.exists());
}

#[test]
fn config_pager_args_are_literal() {
    let scenario = ApplyScenario::new(copy_diff_setup);
    let (script, output) = argv_capture_script(&scenario.env);
    scenario
        .env
        .write_global_config(&config_pager_toml(&script, &["one two", "three", "-R"]));
    let _guard = EnvVarGuard::set([("PAGER", None)]);
    let prompter = QueuePrompter::sequence([ObstructionChoice::ViewDiff, ObstructionChoice::Skip]);

    scenario.run_with_prompter(&prompter);

    let captured = fs::read_to_string(&output).unwrap();
    let lines = captured.lines().collect::<Vec<_>>();
    assert_eq!(&lines[..3], &["one two", "three", "-R"], "{captured}");
    assert!(captured.contains("-old content") && captured.contains("+new content"));
}

#[test]
fn failing_config_pager_fails_the_run_without_falling_back() {
    let scenario = ApplyScenario::new(copy_diff_setup);
    let missing = scenario.env.path("no-such-pager");
    let (env_pager_script, env_pager_output) = capture_script(&scenario.env);
    scenario
        .env
        .write_global_config(&config_pager_toml(&missing, &[]));
    let _guard = EnvVarGuard::set([("PAGER", Some(env_pager_script.to_str().unwrap()))]);
    let prompter = QueuePrompter::sequence([ObstructionChoice::ViewDiff, ObstructionChoice::Skip]);

    let error = scenario.try_run_with_prompter(&prompter).unwrap_err();

    let rendered = format!("{error}");
    assert!(
        rendered.contains("cannot run the configured pager"),
        "{rendered}"
    );
    assert!(!env_pager_output.exists());
}
