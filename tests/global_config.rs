mod common;

use std::fs;
use std::os::unix::fs::symlink;

use common::{ApplyScenario, assert_error_chain, record_of};
use dotrift::commands::apply::ApplyOptions;
use dotrift::state::Kind;

fn minimal_config() -> &'static str {
    r#"
[portal]
"file1" = "file1"
"#
}

fn new_file_scenario() -> ApplyScenario {
    ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        minimal_config()
    })
}

fn assert_nothing_deployed(scenario: &ApplyScenario) {
    assert!(
        fs::symlink_metadata(scenario.target.join("file1")).is_err(),
        "nothing must be deployed when the global config fails"
    );
    assert!(
        scenario.env.database().managed_paths().unwrap().is_empty(),
        "no state record must exist when the global config fails"
    );
}

fn assert_deployed_as_symlink(scenario: &ApplyScenario) {
    let link = scenario.target.join("file1");
    let metadata = fs::symlink_metadata(&link).unwrap();
    assert!(metadata.file_type().is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), scenario.source.join("file1"));
    let record = record_of(&scenario.env, &link).expect("no state record for deployed symlink");
    assert_eq!(record.source_path, scenario.source.join("file1"));
    assert_eq!(record.kind, Kind::Symlink);
}

#[test]
fn missing_global_config_leaves_apply_unchanged() {
    let scenario = new_file_scenario();

    scenario.run();

    assert_deployed_as_symlink(&scenario);
}

#[test]
fn malformed_global_config_fails_run_before_any_change() {
    let scenario = new_file_scenario();
    scenario.env.write_global_config("[[[");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "config.toml");
    assert_nothing_deployed(&scenario);
}

#[test]
fn malformed_global_config_fails_dry_run_before_any_change() {
    let scenario = new_file_scenario();
    scenario.env.write_global_config("[[[");
    let options = ApplyOptions {
        dry_run: true,
        ..ApplyOptions::default()
    };

    let error = scenario.try_run_with_options(options).unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "config.toml");
    assert_nothing_deployed(&scenario);
}

#[test]
fn unknown_root_section_fails_run_before_any_change() {
    let scenario = new_file_scenario();
    scenario.env.write_global_config("[dir1]\nstr = \"str\"\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "unknown field");
    assert_nothing_deployed(&scenario);
}

#[test]
fn unknown_root_key_fails_run_before_any_change() {
    let scenario = new_file_scenario();
    scenario.env.write_global_config("str = \"str\"\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "unknown field");
    assert_nothing_deployed(&scenario);
}

#[test]
fn unknown_pager_property_fails_run_before_any_change() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[pager]\ncommand = \"file1\"\nstr = \"str\"\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "unknown field");
    assert_nothing_deployed(&scenario);
}

#[test]
fn unknown_apply_property_fails_run_before_any_change() {
    let scenario = new_file_scenario();
    scenario.env.write_global_config("[apply]\nstr = \"str\"\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "unknown field");
    assert_nothing_deployed(&scenario);
}

#[test]
fn pager_args_as_string_are_rejected_before_any_change() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[pager]\ncommand = \"file1\"\nargs = \"str\"\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "invalid type");
    assert_nothing_deployed(&scenario);
}

#[test]
fn non_string_pager_arg_is_rejected_before_any_change() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[pager]\ncommand = \"file1\"\nargs = [\"str\", 1]\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "invalid type");
    assert_nothing_deployed(&scenario);
}

#[test]
fn non_bool_replace_identical_is_rejected_before_any_change() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[apply]\nreplace-identical = \"str\"\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "invalid type");
    assert_nothing_deployed(&scenario);
}

#[test]
fn non_string_pager_command_is_rejected_before_any_change() {
    let scenario = new_file_scenario();
    scenario.env.write_global_config("[pager]\ncommand = 1\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "invalid type");
    assert_nothing_deployed(&scenario);
}

#[test]
fn pager_args_without_command_are_rejected_before_any_change() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[pager]\nargs = [\"str\"]\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "missing field");
    assert_nothing_deployed(&scenario);
}

#[test]
fn non_table_pager_is_rejected_before_any_change() {
    let scenario = new_file_scenario();
    scenario.env.write_global_config("pager = \"str\"\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "invalid type");
    assert_nothing_deployed(&scenario);
}

#[test]
fn empty_pager_table_is_rejected_before_any_change() {
    let scenario = new_file_scenario();
    scenario.env.write_global_config("[pager]\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "missing field");
    assert_nothing_deployed(&scenario);
}

#[test]
fn diff_table_with_command_and_args_is_accepted() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[diff]\ncommand = \"file1\"\nargs = [\"-u\"]\n");

    scenario.run();

    assert_deployed_as_symlink(&scenario);
}

#[test]
fn diff_table_alone_is_accepted() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[diff]\ncommand = \"file1\"\n");

    scenario.run();

    assert_deployed_as_symlink(&scenario);
}

#[test]
fn diff_label_placeholders_alone_are_accepted() {
    let scenario = new_file_scenario();
    scenario.env.write_global_config(
        "[diff]\ncommand = \"file1\"\nargs = [\"${target-label}\", \"${source-label}\"]\n",
    );

    scenario.run();

    assert_deployed_as_symlink(&scenario);
}

#[test]
fn diff_embedded_placeholders_are_accepted() {
    let scenario = new_file_scenario();
    scenario.env.write_global_config(
        "[diff]\ncommand = \"file1\"\nargs = [\"--pair=${target}:${source}\"]\n",
    );

    scenario.run();

    assert_deployed_as_symlink(&scenario);
}

#[test]
fn empty_diff_command_counts_as_unset_and_is_not_validated() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[diff]\ncommand = \"\"\nargs = [\"${bogus}\"]\n");

    scenario.run();

    assert_deployed_as_symlink(&scenario);
}

#[test]
fn whitespace_diff_command_counts_as_unset_and_is_not_validated() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[diff]\ncommand = \"   \"\nargs = [\"${bogus}\"]\n");

    scenario.run();

    assert_deployed_as_symlink(&scenario);
}

#[test]
fn unknown_diff_property_fails_run_before_any_change() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[diff]\ncommand = \"file1\"\nstr = \"str\"\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "unknown field");
    assert_nothing_deployed(&scenario);
}

#[test]
fn unknown_diff_key_fails_run_before_any_change() {
    let scenario = new_file_scenario();
    scenario.env.write_global_config("diff = \"str\"\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "invalid type");
    assert_nothing_deployed(&scenario);
}

#[test]
fn empty_diff_table_is_rejected_before_any_change() {
    let scenario = new_file_scenario();
    scenario.env.write_global_config("[diff]\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "missing field");
    assert_nothing_deployed(&scenario);
}

#[test]
fn diff_args_without_command_are_rejected_before_any_change() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[diff]\nargs = [\"-u\"]\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "missing field");
    assert_nothing_deployed(&scenario);
}

#[test]
fn non_string_diff_command_is_rejected_before_any_change() {
    let scenario = new_file_scenario();
    scenario.env.write_global_config("[diff]\ncommand = 1\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "invalid type");
    assert_nothing_deployed(&scenario);
}

#[test]
fn diff_args_as_string_are_rejected_before_any_change() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[diff]\ncommand = \"file1\"\nargs = \"str\"\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "invalid type");
    assert_nothing_deployed(&scenario);
}

#[test]
fn non_string_diff_arg_is_rejected_before_any_change() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[diff]\ncommand = \"file1\"\nargs = [\"str\", 1]\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "invalid type");
    assert_nothing_deployed(&scenario);
}

#[test]
fn unknown_diff_placeholder_fails_run_before_any_change() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[diff]\ncommand = \"file1\"\nargs = [\"${bogus}\"]\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "unknown placeholder");
    assert_nothing_deployed(&scenario);
}

#[test]
fn unterminated_diff_placeholder_fails_run_before_any_change() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[diff]\ncommand = \"file1\"\nargs = [\"${target\"]\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "unterminated placeholder");
    assert_nothing_deployed(&scenario);
}

#[test]
fn diff_args_with_only_target_placeholder_fail_run_before_any_change() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[diff]\ncommand = \"file1\"\nargs = [\"${target}\"]\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "must reference");
    assert_nothing_deployed(&scenario);
}

#[test]
fn diff_args_with_only_source_placeholder_fail_run_before_any_change() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[diff]\ncommand = \"file1\"\nargs = [\"${source}\"]\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "must reference");
    assert_nothing_deployed(&scenario);
}

#[test]
fn pager_and_apply_together_are_accepted() {
    let scenario = new_file_scenario();
    scenario.env.write_global_config(
        "[pager]\ncommand = \"file1\"\n\n[apply]\nreplace-identical = false\n",
    );

    scenario.run();

    assert_deployed_as_symlink(&scenario);
}

#[test]
fn empty_apply_table_is_accepted() {
    let scenario = new_file_scenario();
    scenario.env.write_global_config("[apply]\n");

    scenario.run();

    assert_deployed_as_symlink(&scenario);
}

#[test]
fn explicit_default_apply_value_is_accepted() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[apply]\nreplace-identical = false\n");

    scenario.run();

    assert_deployed_as_symlink(&scenario);
}

#[test]
fn empty_pager_command_is_accepted_as_unset() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[pager]\ncommand = \"\"\n");

    scenario.run();

    assert_deployed_as_symlink(&scenario);
}

#[test]
fn whitespace_pager_command_is_accepted_as_unset() {
    let scenario = new_file_scenario();
    scenario
        .env
        .write_global_config("[pager]\ncommand = \"   \"\n");

    scenario.run();

    assert_deployed_as_symlink(&scenario);
}

#[test]
fn directory_at_config_path_fails_run() {
    let scenario = new_file_scenario();
    let path = scenario.env.path("config-home/dotrift/config.toml");
    fs::create_dir_all(&path).unwrap();

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot read");
    assert_error_chain(&error, "config.toml");
    assert_nothing_deployed(&scenario);
}

#[test]
fn dangling_symlink_at_config_path_counts_as_missing() {
    let scenario = new_file_scenario();
    let path = scenario.env.path("config-home/dotrift/config.toml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    symlink("file1", &path).unwrap();

    scenario.run();

    assert_deployed_as_symlink(&scenario);
}

#[test]
fn global_config_error_surfaces_before_control_file_error() {
    let scenario = ApplyScenario::new(|_source, _target| {
        r#"
[portal]
"file2" = "file1"
"#
    });
    scenario.env.write_global_config("[dir1]\nstr = \"str\"\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "config.toml");
    assert_nothing_deployed(&scenario);
}
