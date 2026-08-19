mod common;

use std::path::Path;

use common::TestEnv;
use dotrift::cli::ProfileCommand;
use test_case::test_case;

fn run_and_take(source: Option<&Path>, command: ProfileCommand) -> String {
    dotrift::capture::clear();
    dotrift::commands::profile::run(source, command).unwrap();
    dotrift::capture::take()
}

fn run_expects_error(source: Option<&Path>, command: ProfileCommand, needle: &str) {
    let error = dotrift::commands::profile::run(source, command).unwrap_err();
    common::assert_error_chain(&error, needle);
}

#[test_case(None, &[], ProfileCommand::List => ""; "list_without_data_file_prints_nothing")]
#[test_case(Some("[variable]\n"), &[], ProfileCommand::List => ""; "list_without_profiles_prints_nothing")]
#[test_case(
    Some("[profile.home]\n[profile.work]\n[profile.editor]\n"),
    &["work"],
    ProfileCommand::List => "editor\nhome\nwork (active)\n";
    "list_marks_active_profiles_in_sorted_order"
)]
#[test_case(
    Some("[profile.a]\n[profile.b]\n"),
    &["a", "b"],
    ProfileCommand::List => "a (active)\nb (active)\n";
    "list_marks_all_active_profiles"
)]
#[test_case(
    Some("[profile.editor]\n"),
    &["gone"],
    ProfileCommand::List => "editor\n";
    "list_hides_stale_active_profiles"
)]
#[test_case(None, &[], ProfileCommand::Show => ""; "show_empty_context_prints_nothing")]
#[test_case(
    Some("[variable]\nv = \"base\"\n[profile.work]\nv = \"over\"\n"),
    &["work"],
    ProfileCommand::Show => "v   over\n";
    "show_active_profile_overrides_base"
)]
#[test_case(
    Some("[variable]\nv = \"base\"\n"),
    &["gone"],
    ProfileCommand::Show => "v   base\n";
    "show_ignores_stale_active_profile"
)]
fn output_matrix(data: Option<&str>, active: &[&str], command: ProfileCommand) -> String {
    let env = TestEnv::new();
    let source = env.source_dir();
    if let Some(contents) = data {
        env.write_data_file(contents);
    }
    for name in active {
        env.database().activate_profile(name).unwrap();
    }
    run_and_take(Some(&source), command)
}

#[test]
fn list_nonexistent_source_directory_errors() {
    let env = TestEnv::new();
    run_expects_error(
        Some(&env.path("missing_source")),
        ProfileCommand::List,
        "does not exist",
    );
}

#[test]
fn activate_defined_profile_succeeds() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file("[profile.work]\n");
    assert_eq!(
        run_and_take(
            Some(&source),
            ProfileCommand::Activate {
                name: "work".into()
            }
        ),
        "profile `work` activated\n"
    );
    let profiles = env.database().active_profiles().unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].0, "work");
}

#[test]
fn activate_undefined_profile_errors() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file("[profile.other]\n");
    run_expects_error(
        Some(&source),
        ProfileCommand::Activate {
            name: "work".into(),
        },
        "profile `work` is not defined",
    );
    assert_eq!(env.database().active_profiles().unwrap(), vec![]);
}

#[test]
fn activate_reactivation_moves_to_precedence_end() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file("[profile.a]\n[profile.b]\n");
    let database = env.database();
    database.activate_profile("a").unwrap();
    database.activate_profile("b").unwrap();
    assert_eq!(
        run_and_take(Some(&source), ProfileCommand::Activate { name: "a".into() }),
        "profile `a` activated\n"
    );
    let names: Vec<String> = database
        .active_profiles()
        .unwrap()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, vec!["b", "a"]);
}

#[test_case(ProfileCommand::List; "list")]
#[test_case(ProfileCommand::Activate { name: "work".into() }; "activate")]
#[test_case(ProfileCommand::Show; "show")]
fn command_requires_source_directory(command: ProfileCommand) {
    let _env = TestEnv::new();
    run_expects_error(None, command, "source directory is required");
}

#[test]
fn deactivate_active_profile_succeeds() {
    let env = TestEnv::new();
    env.database().activate_profile("work").unwrap();
    assert_eq!(
        run_and_take(
            None,
            ProfileCommand::Deactivate {
                name: "work".into()
            },
        ),
        "profile `work` deactivated\n"
    );
    assert_eq!(env.database().active_profiles().unwrap(), vec![]);
}

#[test]
fn deactivate_inactive_profile_errors() {
    let env = TestEnv::new();
    let _database = env.database();
    run_expects_error(
        None,
        ProfileCommand::Deactivate {
            name: "work".into(),
        },
        "profile `work` is not active",
    );
}

#[test]
fn deactivate_stale_profile_without_definition_succeeds() {
    let env = TestEnv::new();
    env.database().activate_profile("gone").unwrap();
    run_and_take(
        None,
        ProfileCommand::Deactivate {
            name: "gone".into(),
        },
    );
    assert_eq!(env.database().active_profiles().unwrap(), vec![]);
}

#[test]
fn show_renders_base_variables() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file(
        "[variable]\neditor = \"vim\"\ntheme = \"dark\"\ncount = 42\nenabled = true\ntags = [\"a\", \"b\"]\nsettings = { lang = \"rust\", indent = 2 }\n",
    );
    insta::assert_snapshot!(run_and_take(Some(&source), ProfileCommand::Show));
}

#[test]
fn show_most_recently_activated_profile_wins() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file(
        "[variable]\nv = \"base\"\n[profile.p1]\nv = \"first\"\n[profile.p2]\nv = \"second\"\n",
    );
    let database = env.database();
    database.activate_profile("p1").unwrap();
    database.activate_profile("p2").unwrap();
    assert_eq!(
        run_and_take(Some(&source), ProfileCommand::Show),
        "v   second\n"
    );
    database.activate_profile("p1").unwrap();
    assert_eq!(
        run_and_take(Some(&source), ProfileCommand::Show),
        "v   first\n"
    );
}

#[test]
fn show_unions_keys_across_multiple_active_profiles() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file(
        "[variable]\nbase = \"base\"\n[profile.a]\nbase = \"a\"\nfrom_a = \"A\"\n[profile.b]\nbase = \"b\"\nfrom_b = \"B\"\n[profile.c]\nbase = \"c\"\nfrom_c = \"C\"\n",
    );
    let database = env.database();
    database.activate_profile("a").unwrap();
    database.activate_profile("b").unwrap();
    database.activate_profile("c").unwrap();
    assert_eq!(
        run_and_take(Some(&source), ProfileCommand::Show),
        "base     c\nfrom_a   A\nfrom_b   B\nfrom_c   C\n"
    );
}

#[test]
fn all_state_transitions_round_trip() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file("[variable]\nv = \"base\"\n[profile.work]\nv = \"over\"\n");
    let database = env.database();
    database.activate_profile("work").unwrap();
    assert_eq!(
        run_and_take(Some(&source), ProfileCommand::Show),
        "v   over\n"
    );
    assert_eq!(
        run_and_take(
            None,
            ProfileCommand::Deactivate {
                name: "work".into()
            },
        ),
        "profile `work` deactivated\n"
    );
    assert_eq!(
        run_and_take(Some(&source), ProfileCommand::Show),
        "v   base\n"
    );
}
