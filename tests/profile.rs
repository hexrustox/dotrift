mod common;

use std::path::Path;

use common::{TestEnv, assert_error_chain, snapshot_settings, test_name};
use dotrift::cli::ProfileCommand;
use dotrift::commands::profile;

fn active_names(env: &TestEnv) -> Vec<String> {
    env.database()
        .active_profiles()
        .expect("cannot read active profiles")
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

fn run_list(env: &TestEnv, source: &Path) -> String {
    dotrift::report::clear();
    profile::run(Some(source), ProfileCommand::List, env.env(), false)
        .expect("profile list failed");
    dotrift::report::take_output()
}

fn run_show(env: &TestEnv, source: &Path) -> String {
    dotrift::report::clear();
    profile::run(Some(source), ProfileCommand::Show, env.env(), false)
        .expect("profile show failed");
    dotrift::report::take_output()
}

fn run_activate(env: &TestEnv, source: &Path, name: &str) -> String {
    dotrift::report::clear();
    profile::run(
        Some(source),
        ProfileCommand::Activate {
            name: name.to_string(),
        },
        env.env(),
        false,
    )
    .expect("profile activate failed");
    dotrift::report::take_output()
}

fn run_deactivate(env: &TestEnv, source: Option<&Path>, name: &str) -> String {
    dotrift::report::clear();
    profile::run(
        source,
        ProfileCommand::Deactivate {
            name: name.to_string(),
        },
        env.env(),
        false,
    )
    .expect("profile deactivate failed");
    dotrift::report::take_output()
}

#[test]
fn profile_list_reports_nothing_without_data_file() {
    let env = TestEnv::new();
    let source = env.source_dir();

    assert_eq!(run_list(&env, &source), "");
}

#[test]
fn profile_list_reports_nothing_without_defined_profiles() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file("[variable]\nstr = \"str\"\n");

    assert_eq!(run_list(&env, &source), "");
}

#[test]
fn profile_list_annotates_active_profile_in_sorted_output() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file(
        "[profile.profile2]\nstr = \"content2\"\n\n[profile.profile1]\nstr = \"content1\"\n",
    );
    run_activate(&env, &source, "profile1");

    assert_eq!(run_list(&env, &source), "profile1 (active)\nprofile2\n");
}

#[test]
fn profile_list_annotates_every_active_profile() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file(
        "[profile.profile1]\nstr = \"content1\"\n\n[profile.profile2]\nstr = \"content2\"\n",
    );
    run_activate(&env, &source, "profile1");
    run_activate(&env, &source, "profile2");

    assert_eq!(
        run_list(&env, &source),
        "profile1 (active)\nprofile2 (active)\n"
    );
}

#[test]
fn profile_list_omits_active_profile_without_definition() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file(
        "[profile.profile1]\nstr = \"content1\"\n\n[profile.profile2]\nstr = \"content2\"\n",
    );
    run_activate(&env, &source, "profile1");
    env.write_data_file("[profile.profile2]\nstr = \"content2\"\n");

    assert_eq!(run_list(&env, &source), "profile2\n");
}

#[test]
fn profile_show_reports_nothing_without_data_file() {
    let env = TestEnv::new();
    let source = env.source_dir();

    assert_eq!(run_show(&env, &source), "");
}

#[test]
fn profile_show_resolves_values_through_active_override() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file("[variable]\nstr = \"str\"\n\n[profile.profile1]\nstr = \"content1\"\n");

    assert_eq!(run_show(&env, &source), "str   str\n");
    run_activate(&env, &source, "profile1");

    assert_eq!(run_show(&env, &source), "str   content1\n");
}

#[test]
fn profile_show_ignores_active_profile_without_definition() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file("[variable]\nstr = \"str\"\n\n[profile.profile1]\nstr = \"content1\"\n");
    run_activate(&env, &source, "profile1");
    env.write_data_file("[variable]\nstr = \"str\"\n");

    assert_eq!(run_show(&env, &source), "str   str\n");
}

#[test]
fn profile_list_with_nonexistent_source_errors() {
    let env = TestEnv::new();
    let missing = env.path("missing");

    dotrift::report::clear();
    let error = profile::run(
        Some(missing.as_path()),
        ProfileCommand::List,
        env.env(),
        false,
    )
    .unwrap_err();

    assert_error_chain(&error, "source directory");
    assert_error_chain(&error, "does not exist");
}

#[test]
fn activating_defined_profile_confirms_and_records_activation() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file("[profile.profile1]\nstr = \"content1\"\n");

    let output = run_activate(&env, &source, "profile1");

    assert_eq!(output, "profile `profile1` activated\n");
    assert_eq!(active_names(&env), vec!["profile1".to_string()]);
}

#[test]
fn activating_undefined_profile_errors_without_activation() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file("[profile.profile1]\nstr = \"content1\"\n");

    dotrift::report::clear();
    let error = profile::run(
        Some(source.as_path()),
        ProfileCommand::Activate {
            name: "profile2".to_string(),
        },
        env.env(),
        false,
    )
    .unwrap_err();

    assert_error_chain(&error, "profile `profile2` is not defined");
    assert!(active_names(&env).is_empty());
    assert_eq!(dotrift::report::take_output(), "");
}

#[test]
fn reactivating_profile_moves_it_to_end_of_precedence() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file(
        "[profile.profile1]\nstr = \"content1\"\n\n[profile.profile2]\nstr = \"content2\"\n",
    );
    run_activate(&env, &source, "profile1");
    run_activate(&env, &source, "profile2");
    run_activate(&env, &source, "profile1");

    assert_eq!(
        active_names(&env),
        vec!["profile2".to_string(), "profile1".to_string()]
    );
}

#[test]
fn profile_list_without_source_errors() {
    let env = TestEnv::new();

    dotrift::report::clear();
    let error = profile::run(None, ProfileCommand::List, env.env(), false).unwrap_err();

    assert_error_chain(&error, "source directory is required");
}

#[test]
fn profile_activate_without_source_errors() {
    let env = TestEnv::new();

    dotrift::report::clear();
    let error = profile::run(
        None,
        ProfileCommand::Activate {
            name: "profile1".to_string(),
        },
        env.env(),
        false,
    )
    .unwrap_err();

    assert_error_chain(&error, "source directory is required");
}

#[test]
fn profile_show_without_source_errors() {
    let env = TestEnv::new();

    dotrift::report::clear();
    let error = profile::run(None, ProfileCommand::Show, env.env(), false).unwrap_err();

    assert_error_chain(&error, "source directory is required");
}

#[test]
fn deactivating_active_profile_confirms_and_clears() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file("[profile.profile1]\nstr = \"content1\"\n");
    run_activate(&env, &source, "profile1");

    let output = run_deactivate(&env, None, "profile1");

    assert_eq!(output, "profile `profile1` deactivated\n");
    assert!(active_names(&env).is_empty());
}

#[test]
fn deactivating_inactive_profile_errors() {
    let env = TestEnv::new();
    let _source = env.source_dir();

    dotrift::report::clear();
    let error = profile::run(
        None,
        ProfileCommand::Deactivate {
            name: "profile1".to_string(),
        },
        env.env(),
        false,
    )
    .unwrap_err();

    assert_error_chain(&error, "profile `profile1` is not active");
}

#[test]
fn deactivating_stale_profile_clears_without_definition() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file("[profile.profile1]\nstr = \"content1\"\n");
    run_activate(&env, &source, "profile1");
    env.write_data_file("[variable]\nstr = \"str\"\n");

    let output = run_deactivate(&env, Some(source.as_path()), "profile1");

    assert_eq!(output, "profile `profile1` deactivated\n");
    assert!(active_names(&env).is_empty());
}

#[test]
fn profile_show_formats_all_base_variable_types() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file(
        "[variable]\nflag = true\nlist1 = [1, \"str\", true]\nmap1 = { num = 1, str = \"str\" }\nnum = 1\nstr = \"str\"\n",
    );

    let captured = run_show(&env, &source);

    snapshot_settings(&env).bind(|| {
        insta::assert_snapshot!(test_name(), captured);
    });
}

#[test]
fn profile_show_prefers_most_recently_activated_profile() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file(
        "[variable]\nstr = \"str\"\n\n[profile.profile1]\nstr = \"content1\"\n\n[profile.profile2]\nstr = \"content2\"\n",
    );
    run_activate(&env, &source, "profile1");
    run_activate(&env, &source, "profile2");

    assert_eq!(run_show(&env, &source), "str   content2\n");
}

#[test]
fn profile_show_unions_variables_across_active_profiles() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file("[profile.profile1]\nstr = \"str\"\n\n[profile.profile2]\nnum = 1\n");
    run_activate(&env, &source, "profile1");
    run_activate(&env, &source, "profile2");

    assert_eq!(run_show(&env, &source), "num   1\nstr   str\n");
}

#[test]
fn activate_then_deactivate_round_trips_show_output() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file("[variable]\nstr = \"str\"\n\n[profile.profile1]\nstr = \"content1\"\n");

    let before = run_show(&env, &source);
    run_activate(&env, &source, "profile1");
    let during = run_show(&env, &source);
    run_deactivate(&env, None, "profile1");
    let after = run_show(&env, &source);

    assert_eq!(before, "str   str\n");
    assert_eq!(during, "str   content1\n");
    assert_eq!(after, before);
}
