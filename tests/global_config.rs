mod common;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use common::{ApplyScenario, assert_error_chain, prompt_count};
use dotrift::commands::apply::ApplyOptions;
use dotrift::obstruction_interaction::{ObstructionChoice, test_hooks::set_prompt_choice};
use test_case::test_case;

fn obstruction_setup(source: &Path, target: &Path) -> &'static str {
    fs::write(source.join("file.txt"), b"new").unwrap();
    fs::write(target.join("target.txt"), b"old").unwrap();
    "[portal]\n\"file.txt\" = \"target.txt\"\n"
}

#[test]
fn missing_config_file_leaves_apply_behavior_unchanged() {
    let scenario = ApplyScenario::new(obstruction_setup);
    scenario.env.write_global_config("");

    set_prompt_choice(ObstructionChoice::Skip);
    let status = scenario.try_run().expect("apply failed");

    assert_eq!(status, dotrift::ExitStatus::Skipped);
    assert_eq!(
        fs::read(scenario.target.join("target.txt")).unwrap(),
        b"old"
    );
    assert_eq!(prompt_count(), 1);
}

#[test]
fn malformed_toml_fails_the_run_before_any_change() {
    let scenario = ApplyScenario::new(obstruction_setup);
    scenario.env.write_global_config("[pager\ncommand =");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_eq!(
        fs::read(scenario.target.join("target.txt")).unwrap(),
        b"old"
    );
    assert_eq!(prompt_count(), 0);
}

#[test]
fn malformed_toml_fails_a_dry_run() {
    let scenario = ApplyScenario::new(obstruction_setup);
    scenario.env.write_global_config("[pager\ncommand =");

    let error = scenario
        .try_run_with_options(ApplyOptions {
            dry_run: true,
            ..Default::default()
        })
        .unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_eq!(
        fs::read(scenario.target.join("target.txt")).unwrap(),
        b"old"
    );
    assert_eq!(prompt_count(), 0);
}

#[test_case("[bogus]\nkey = 1" ; "unknown_root_section_rejected")]
#[test_case("stray = 1" ; "unknown_root_key_rejected")]
#[test_case("[pager]\ncommand = \"less\"\nbogus = 1" ; "unknown_pager_property_rejected")]
#[test_case("[apply]\nreplace-identical = false\nbogus = 1" ; "unknown_apply_property_rejected")]
#[test_case("[pager]\nargs = \"-R\"" ; "args_not_an_array_of_strings_rejected")]
#[test_case("[pager]\ncommand = \"less\"\nargs = [1]" ; "non_string_arg_rejected")]
#[test_case("[apply]\nreplace-identical = \"yes\"" ; "replace_identical_not_a_bool_rejected")]
#[test_case("[pager]\ncommand = 1" ; "command_not_a_string_rejected")]
#[test_case("[pager]\nargs = [\"-R\"]" ; "missing_command_rejected")]
#[test_case("pager = \"less\"" ; "pager_not_a_table_rejected")]
#[test_case("[pager]\n" ; "empty_pager_table_rejected")]
fn invalid_config_fails_the_run_before_any_change(toml: &str) {
    let scenario = ApplyScenario::new(obstruction_setup);
    scenario.env.write_global_config(toml);

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_eq!(
        fs::read(scenario.target.join("target.txt")).unwrap(),
        b"old"
    );
    assert_eq!(prompt_count(), 0);
}

#[test_case("[pager]\ncommand = \"less\"\n[apply]\nreplace-identical = false" ; "both_sections_accepted")]
#[test_case("[apply]\n" ; "empty_apply_table_accepted")]
#[test_case("[apply]\nreplace-identical = false" ; "explicit_default_accepted")]
#[test_case("[pager]\ncommand = \"\"\n" ; "empty_command_accepted")]
#[test_case("[pager]\ncommand = \"   \"" ; "whitespace_command_accepted")]
fn valid_config_is_accepted(toml: &str) {
    let scenario = ApplyScenario::new(obstruction_setup);
    scenario.env.write_global_config(toml);

    set_prompt_choice(ObstructionChoice::Skip);
    let status = scenario.try_run().expect("apply failed");

    assert_eq!(status, dotrift::ExitStatus::Skipped);
    assert_eq!(prompt_count(), 1);
}

#[test]
fn config_path_being_a_directory_fails_the_run() {
    let scenario = ApplyScenario::new(obstruction_setup);
    scenario.env.write_global_config("");
    let path = scenario.env.path("config-home/dotrift/config.toml");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot read");
    assert_eq!(
        fs::read(scenario.target.join("target.txt")).unwrap(),
        b"old"
    );
    assert_eq!(prompt_count(), 0);
}

#[test]
fn dangling_symlink_at_config_path_counts_as_missing() {
    let scenario = ApplyScenario::new(obstruction_setup);
    scenario.env.write_global_config("");
    let path = scenario.env.path("config-home/dotrift/config.toml");
    fs::remove_file(&path).unwrap();
    symlink("nowhere", &path).unwrap();

    set_prompt_choice(ObstructionChoice::Skip);
    let status = scenario.try_run().expect("apply failed");

    assert_eq!(status, dotrift::ExitStatus::Skipped);
    assert_eq!(prompt_count(), 1);
}

#[test]
fn config_read_before_control_files() {
    let scenario = ApplyScenario::new(obstruction_setup);
    scenario.env.write_global_config("[bogus]");
    scenario.write_config("[portal]\n\"file.txt\" = \"missing-source.txt\"\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
}
