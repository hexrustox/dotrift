mod common;

use std::fs;

use common::TestEnv;
use dotrift::capture;
use dotrift::cli::ProfileCommand;
use dotrift::commands::profile;

fn run(env: &TestEnv, command: ProfileCommand) -> String {
    capture::clear();
    profile::run(Some(env.path("source").as_path()), command).expect("profile command failed");
    capture::take()
}

fn source_with_data(env: &TestEnv, data: &str) {
    fs::create_dir_all(env.path("source")).unwrap();
    fs::write(env.path("source/dotrift_data.toml"), data).unwrap();
}

#[test]
fn list_sorts_profiles_and_marks_only_defined_active_profiles() {
    let env = TestEnv::new();
    source_with_data(
        &env,
        "[profile.zeta]\nvalue = 1\n[profile.alpha]\nvalue = 2",
    );
    run(
        &env,
        ProfileCommand::Activate {
            name: "zeta".into(),
        },
    );
    let database = env.database();
    database.activate_profile("stale").unwrap();

    assert_eq!(run(&env, ProfileCommand::List), "alpha\nzeta (active)\n");
}

#[test]
fn reactivation_moves_profile_to_latest_precedence() {
    let env = TestEnv::new();
    source_with_data(
        &env,
        "[variable]\nvalue = \"base\"\n[profile.a]\nvalue = \"a\"\n[profile.b]\nvalue = \"b\"",
    );
    run(&env, ProfileCommand::Activate { name: "a".into() });
    run(&env, ProfileCommand::Activate { name: "b".into() });
    run(&env, ProfileCommand::Activate { name: "a".into() });

    assert!(
        run(&env, ProfileCommand::Show)
            .lines()
            .any(|line| line.ends_with("a"))
    );
}

#[test]
fn deactivate_removes_stale_profile_without_reading_data_file() {
    let env = TestEnv::new();
    fs::create_dir_all(env.path("source")).unwrap();
    let database = env.database();
    database.activate_profile("removed").unwrap();

    assert_eq!(
        run(
            &env,
            ProfileCommand::Deactivate {
                name: "removed".into()
            }
        ),
        "profile `removed` deactivated\n"
    );
}

#[test]
fn show_uses_canonical_nested_values() {
    let env = TestEnv::new();
    source_with_data(
        &env,
        "[variable]\nplain = \"text\"\nitems = [\"x\", 2]\nsettings = { enabled = true }",
    );

    assert_eq!(
        run(&env, ProfileCommand::Show),
        "items                  [\"x\", 2]\nplain                  text\nsettings               {\"enabled\": true}\n"
    );
}

#[test]
fn unsupported_values_are_rejected() {
    let env = TestEnv::new();
    source_with_data(&env, "[variable]\nnumber = 1.5");

    assert!(profile::run(Some(env.path("source").as_path()), ProfileCommand::Show).is_err());
}
