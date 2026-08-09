mod common;

use std::fs;

use common::TestEnv;
use dotrift::capture;
use dotrift::cli::ProfileCommand;
use dotrift::commands::profile;
use test_case::test_case;

fn run(env: &TestEnv, command: ProfileCommand) -> String {
    capture::clear();
    profile::run(Some(env.path("source").as_path()), command).expect("profile command failed");
    capture::take()
}

fn source_with_data(env: &TestEnv, data: &str) {
    fs::create_dir_all(env.path("source")).unwrap();
    fs::write(env.path("source/dotrift_data.toml"), data).unwrap();
}

#[test_case(|env: &TestEnv| {
    source_with_data(
        env,
        r#"[profile.b]
[profile.a]"#,
    );
    run(env, ProfileCommand::Activate { name: "b".into() });
    env.database().activate_profile("stale").unwrap();
    run(env, ProfileCommand::List)
}; "list_sorts_profiles_and_marks_only_defined_active_profiles")]
#[test_case(|env: &TestEnv| {
    source_with_data(
        env,
        r#"[variable]
value = ""
[profile.a]
value = "a"
[profile.b]
value = "b""#,
    );
    run(env, ProfileCommand::Activate { name: "a".into() });
    run(env, ProfileCommand::Activate { name: "b".into() });
    run(env, ProfileCommand::Activate { name: "a".into() });
    run(env, ProfileCommand::Show)
}; "reactivation_moves_profile_to_latest_precedence")]
#[test_case(|env: &TestEnv| {
    fs::create_dir_all(env.path("source")).unwrap();
    env.database().activate_profile("removed").unwrap();
    run(env, ProfileCommand::Deactivate { name: "removed".into() })
}; "deactivate_removes_stale_profile_without_reading_data_file")]
#[test_case(|env: &TestEnv| {
    source_with_data(
        env,
        r#"[variable]
plain = "text"
items = ["x", 2]
settings = { enabled = true }"#,
    );
    run(env, ProfileCommand::Show)
}; "show_uses_canonical_nested_values")]
fn capture_and_snapshot(setup: impl FnOnce(&TestEnv) -> String) {
    let env = TestEnv::new();
    let test_name = std::thread::current().name().unwrap().replace(":", "_");
    insta::assert_snapshot!(test_name, setup(&env));
}
