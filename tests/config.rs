mod common;

use std::collections::BTreeMap;
use std::fs;

use common::{ApplyScenario, Prompt, TestEnv, assert_error_chain, record_of};
use dotrift::commands::apply::ApplyOptions;
use dotrift::config::{self, DeployType};
use dotrift::state::{Kind, hash_bytes};
use templater::value::Value;
use test_case::test_case;

#[test]
fn config_read_assembles_entries_and_variable_context() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        fs::write(
            source.join("dotrift_data.toml"),
            "[variable]\nstr = \"str\"\nnum = 1\n",
        )
        .unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "file2"

[rule]
"file2" = { type = "copy", mode = "600" }
"#
    });

    let deployment = config::read(
        &scenario.source,
        Some(scenario.target.clone()),
        scenario.env.env(),
        false,
    )
    .unwrap();

    assert_eq!(deployment.target_directory, scenario.target);
    assert_eq!(deployment.entries.len(), 2);
    let first = &deployment.entries[0];
    assert_eq!(first.source_path, scenario.source.join("file1"));
    assert_eq!(first.target_path, scenario.target.join("file1"));
    assert_eq!(first.deploy_type, DeployType::Symlink);
    assert_eq!(first.mode, None);
    let second = &deployment.entries[1];
    assert_eq!(second.source_path, scenario.source.join("file2"));
    assert_eq!(second.target_path, scenario.target.join("file2"));
    assert_eq!(second.deploy_type, DeployType::Copy);
    assert_eq!(second.mode.map(u32::from), Some(0o600));
    assert_eq!(
        deployment.variable_context.get("str"),
        Some(&Value::Str("str".into()))
    );
    assert_eq!(deployment.variable_context.get("num"), Some(&Value::Int(1)));

    scenario.run();

    let record =
        record_of(&scenario.env, &scenario.target.join("file2")).expect("copy must be recorded");
    assert_eq!(
        record.content_hash.as_deref(),
        Some(hash_bytes(b"content2").as_str())
    );
}

#[test]
fn symlink_effective_type_drops_wildcard_mode_with_warning() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"*" = { type = "copy", mode = "600" }
"file1" = { type = "symlink" }
"#
    });
    dotrift::report::clear();

    scenario.run();

    let warnings = dotrift::report::take_errors();
    assert!(
        warnings.contains("ignoring `mode`") && warnings.contains("file1"),
        "expected a mode-drop warning naming the target, got: {warnings:?}"
    );
    let link = scenario.target.join("file1");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let record = record_of(&scenario.env, &link).expect("symlink must be recorded");
    assert_eq!(record.kind, Kind::Symlink);
}

#[test]
fn active_profile_overrides_base_variable_in_config_and_context() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "{{ str }}"
"#
    });
    scenario
        .env
        .write_data_file("[variable]\nstr = \"file1\"\n\n[profile.profile1]\nstr = \"file2\"\n");
    scenario
        .env
        .database()
        .activate_profile("profile1")
        .unwrap();

    let deployment = config::read(
        &scenario.source,
        Some(scenario.target.clone()),
        scenario.env.env(),
        false,
    )
    .unwrap();
    assert_eq!(
        deployment.variable_context.get("str"),
        Some(&Value::Str("file2".into()))
    );
    assert_eq!(deployment.entries.len(), 1);
    assert_eq!(
        deployment.entries[0].target_path,
        scenario.target.join("file2")
    );

    scenario.run();

    let link = scenario.target.join("file2");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(fs::symlink_metadata(scenario.target.join("file1")).is_err());
}

#[test]
fn active_profile_without_definition_is_ignored() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "{{ str }}\n").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "template" }
"#
    });
    scenario.env.write_data_file("[variable]\nstr = \"str\"\n");
    scenario
        .env
        .database()
        .activate_profile("profile1")
        .unwrap();

    scenario.run();

    assert_eq!(fs::read(scenario.target.join("file1")).unwrap(), b"str\n");
}

#[test]
fn missing_template_variable_fails_before_toml_parsing() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"{{ missing }}" = "file1"
"#
    });

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "undefined variable");
    assert!(
        !error
            .chain()
            .any(|cause| cause.to_string().contains("cannot parse")),
        "missing variables must fail before TOML parsing, got: {error:?}"
    );
    assert!(fs::symlink_metadata(scenario.target.join("file1")).is_err());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn rendered_config_values_validated_like_literals() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "{{ str }}"
"#
    });
    scenario
        .env
        .write_data_file("[variable]\nstr = \"../file1\"\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "invalid portal target path");
    assert!(fs::symlink_metadata(scenario.target.join("file1")).is_err());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn unreadable_ignore_file_fails_as_configuration_error() {
    // Probing a mode-000 file in a throw-away directory: environments that
    // bypass permission checks (sandboxed shells, root) cannot express
    // "unreadable" this way, so the scenario would exercise nothing.
    let gate = tempfile::TempDir::new().unwrap();
    let probe = gate.path().join("file1");
    fs::write(&probe, "content1").unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&probe, fs::Permissions::from_mode(0o000)).unwrap();
    }
    if fs::File::open(&probe).is_ok() {
        eprintln!("skipping: ambient permission model ignores mode 000");
        return;
    }

    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join(".dotriftignore"), "/dir2\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(
                source.join(".dotriftignore"),
                fs::Permissions::from_mode(0o000),
            )
            .unwrap();
        }
        r#"
[portal]
"file1" = "file1"
"#
    });

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, ".dotriftignore");
    assert!(fs::symlink_metadata(scenario.target.join("file1")).is_err());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn unreadable_ignore_directory_fails_as_configuration_error() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::create_dir(source.join(".dotriftignore")).unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, ".dotriftignore");
    assert!(fs::symlink_metadata(scenario.target.join("file1")).is_err());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn malformed_data_file_names_path_in_error() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });
    scenario.env.write_data_file("[variable\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "dotrift_data.toml");
    assert!(fs::symlink_metadata(scenario.target.join("file1")).is_err());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn float_variable_value_rejected() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });
    scenario.env.write_data_file("[variable]\nnum = 1.5\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "dotrift_data.toml");
    assert!(fs::symlink_metadata(scenario.target.join("file1")).is_err());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn empty_variable_key_rejected() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });
    scenario.env.write_data_file("[variable]\n\"\" = \"str\"\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "empty key in `[variable]`");
    assert!(fs::symlink_metadata(scenario.target.join("file1")).is_err());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn empty_profile_binding_key_rejected() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });
    scenario
        .env
        .write_data_file("[profile.profile1]\n\"\" = \"str\"\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "empty key in `[profile.profile1]`");
    assert!(fs::symlink_metadata(scenario.target.join("file1")).is_err());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn empty_profile_name_rejected() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });
    scenario
        .env
        .write_data_file("[profile.\"\"]\nstr = \"str\"\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "empty profile name");
    assert!(fs::symlink_metadata(scenario.target.join("file1")).is_err());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn missing_config_names_path_in_error() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });
    fs::remove_file(scenario.source.join("dotrift.toml")).unwrap();

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "dotrift.toml");
    assert!(fs::symlink_metadata(scenario.target.join("file1")).is_err());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn malformed_toml_names_config_in_error() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });
    scenario.write_config("[portal\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot parse");
    assert_error_chain(&error, "dotrift.toml");
    assert!(fs::symlink_metadata(scenario.target.join("file1")).is_err());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn unknown_root_key_rejected() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
unknown-key = "str"

[portal]
"file1" = "file1"
"#
    });

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "unknown field");
    assert!(fs::symlink_metadata(scenario.target.join("file1")).is_err());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn double_slash_rule_key_rejected() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"dir1//file1" = { type = "copy" }
"#
    });

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "invalid rule path");
    assert!(fs::symlink_metadata(scenario.target.join("file1")).is_err());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn trailing_slash_rule_key_rejected() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1/" = { type = "copy" }
"#
    });

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "invalid rule path");
    assert!(fs::symlink_metadata(scenario.target.join("file1")).is_err());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn literal_file_targeting_root_rejected() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "."
"#
    });

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot target");
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn glob_destination_colliding_with_literal_target_rejected() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        r#"
[portal]
"*" = "."
"file1" = "file1"
"#
    });

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "collision");
    assert_error_chain(&error, "file1");
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn dot_slash_prefixed_value_colliding_with_plain_target_rejected() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "./file1"
"#
    });

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "collision");
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn glob_destination_structurally_conflicting_with_ancestor_rejected() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        r#"
[portal]
"*" = "dir1"
"file1" = "dir1"
"#
    });

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "structural conflict");
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn cross_portal_collision_reports_location() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "file1"
"#
    });

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "collision at");
    assert_error_chain(&error, "file2");
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn relative_target_directory_rejected() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
target-directory = "dir1"

[portal]
"file1" = "file1"
"#
    });

    let error = dotrift::commands::apply::run_with_options_and_prompter(
        &scenario.source,
        None,
        ApplyOptions::default(),
        scenario.env.env(),
        false,
        &Prompt::never(),
    )
    .unwrap_err();

    assert_error_chain(&error, "must be an absolute path");
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn explicit_target_override_beats_configured_target() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });
    let configured = scenario.env.path("configured-target");
    scenario.write_config(&format!(
        "target-directory = \"{}\"\n\n[portal]\n\"file1\" = \"file1\"\n",
        configured.display()
    ));

    scenario.run();

    let link = scenario.target.join("file1");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_link(&link).unwrap(), scenario.source.join("file1"));
    assert!(fs::symlink_metadata(configured.join("file1")).is_err());
}

#[test]
fn configured_target_used_without_override() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });
    let configured = scenario.env.path("configured-target");
    scenario.write_config(&format!(
        "target-directory = \"{}\"\n\n[portal]\n\"file1\" = \"file1\"\n",
        configured.display()
    ));

    dotrift::commands::apply::run_with_options_and_prompter(
        &scenario.source,
        None,
        ApplyOptions::default(),
        scenario.env.env(),
        false,
        &Prompt::never(),
    )
    .unwrap();

    let link = configured.join("file1");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_link(&link).unwrap(), scenario.source.join("file1"));
}

#[test]
fn home_directory_fallback_without_override_or_configured_target() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });
    let home = scenario.env.path("home-target");
    fs::create_dir_all(&home).unwrap();
    let _guard = scenario
        .env
        .set_vars([("HOME", Some(home.to_str().unwrap()))]);

    let deployment = config::read(&scenario.source, None, scenario.env.env(), false).unwrap();

    assert_eq!(deployment.target_directory, home);
}

#[test]
fn ignore_file_excludes_entries_and_control_files_never_deploy() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        fs::write(source.join(".dotriftignore"), "file2\n").unwrap();
        r#"
[portal]
"*" = "."
"#
    });

    scenario.run();

    assert!(
        fs::symlink_metadata(scenario.target.join("file1"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(fs::symlink_metadata(scenario.target.join("file2")).is_err());
    assert!(fs::symlink_metadata(scenario.target.join("dotrift.toml")).is_err());
    assert!(fs::symlink_metadata(scenario.target.join(".dotriftignore")).is_err());
    let records = scenario.env.database().managed_paths().unwrap();
    assert_eq!(records.len(), 1);
    assert!(record_of(&scenario.env, &scenario.target.join("file1")).is_some());
}

#[test]
fn negation_pattern_reincludes_ignored_target() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        fs::write(source.join(".dotriftignore"), "file*\n!file1\n").unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "file2"
"#
    });

    scenario.run();

    assert!(
        fs::symlink_metadata(scenario.target.join("file1"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(fs::symlink_metadata(scenario.target.join("file2")).is_err());
    assert!(record_of(&scenario.env, &scenario.target.join("file1")).is_some());
    assert!(record_of(&scenario.env, &scenario.target.join("file2")).is_none());
}

#[test]
fn negated_pattern_reincludes_control_file() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join(".dotriftignore"), "!dotrift.toml\n").unwrap();
        r#"
[portal]
"*" = "."
"#
    });

    scenario.run();

    for name in ["file1", "dotrift.toml"] {
        let link = scenario.target.join(name);
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink(),
            "{name} must deploy"
        );
        assert_eq!(fs::read_link(&link).unwrap(), scenario.source.join(name));
    }
}

#[test]
fn nested_control_filename_deploys_when_mapped_explicitly() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::create_dir_all(source.join("dir1")).unwrap();
        fs::write(source.join("dir1/dotrift.toml"), "content1").unwrap();
        r#"
[portal]
"dir1/dotrift.toml" = "file1"
"#
    });

    scenario.run();

    let link = scenario.target.join("file1");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_link(&link).unwrap(),
        scenario.source.join("dir1/dotrift.toml")
    );
    let record = record_of(&scenario.env, &link).expect("nested control file must be recorded");
    assert_eq!(
        record.source_path,
        scenario.source.join("dir1/dotrift.toml")
    );
}

#[test]
fn ignoring_colliding_targets_skips_collision_validation() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::write(source.join("file2"), "content2").unwrap();
        fs::write(source.join(".dotriftignore"), "file1\n").unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "file1"
"#
    });

    scenario.run();

    assert!(fs::symlink_metadata(scenario.target.join("file1")).is_err());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn directory_only_ignore_pattern_inert_for_files() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::create_dir_all(source.join("dir1")).unwrap();
        fs::write(source.join("dir1/file1"), "content1").unwrap();
        fs::write(source.join(".dotriftignore"), "dir1/\n").unwrap();
        r#"
[portal]
"dir1" = "dir1"
"#
    });

    scenario.run();

    let link = scenario.target.join("dir1/file1");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(record_of(&scenario.env, &link).is_some());
}

#[test]
fn profile_overlay_replaces_whole_value_without_merging() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });
    scenario.env.write_data_file(
        "[variable]\nstr = \"str\"\n\n[variable.settings]\nnum = 1\nnum2 = 2\n\n[profile.profile1.settings]\nnum = 3\n",
    );

    let base = config::read(
        &scenario.source,
        Some(scenario.target.clone()),
        scenario.env.env(),
        false,
    )
    .unwrap();
    assert_eq!(
        base.variable_context.get("settings"),
        Some(&Value::Map(BTreeMap::from([
            ("num".to_string(), Value::Int(1)),
            ("num2".to_string(), Value::Int(2)),
        ])))
    );

    scenario
        .env
        .database()
        .activate_profile("profile1")
        .unwrap();
    let overlaid = config::read(
        &scenario.source,
        Some(scenario.target.clone()),
        scenario.env.env(),
        false,
    )
    .unwrap();
    assert_eq!(
        overlaid.variable_context.get("settings"),
        Some(&Value::Map(BTreeMap::from([(
            "num".to_string(),
            Value::Int(3)
        )])))
    );
}

#[test]
fn missing_source_directory_rejected() {
    let env = TestEnv::new();
    let target = env.target_dir();
    let missing = env.path("dir1");

    let error = dotrift::commands::apply::run_with_options_and_prompter(
        &missing,
        Some(target),
        ApplyOptions::default(),
        env.env(),
        false,
        &Prompt::never(),
    )
    .unwrap_err();

    assert_error_chain(&error, "does not exist");
}

#[test]
fn file_source_path_rejected_as_not_a_directory() {
    let env = TestEnv::new();
    let target = env.target_dir();
    let source = env.path("file1");
    fs::write(&source, "content1").unwrap();

    let error = dotrift::commands::apply::run_with_options_and_prompter(
        &source,
        Some(target),
        ApplyOptions::default(),
        env.env(),
        false,
        &Prompt::never(),
    )
    .unwrap_err();

    assert_error_chain(&error, "is not a directory");
}

#[test]
fn target_inside_source_rejected_as_overlapping() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });

    let error = dotrift::commands::apply::run_with_options_and_prompter(
        &scenario.source,
        Some(scenario.source.join("dir1")),
        ApplyOptions::default(),
        scenario.env.env(),
        false,
        &Prompt::never(),
    )
    .unwrap_err();

    assert_error_chain(&error, "overlap");
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test_case("-1" ; "negative_integer_mode_is_invalid")]
#[test_case("4294967296" ; "integer_mode_above_u32_is_invalid")]
#[test_case("1000" ; "integer_mode_above_octal_range_is_invalid")]
fn an_integer_mode_outside_the_octal_range_is_invalid(toml_mode: &str) {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "copy" }
"#
    });
    scenario.write_config(&format!(
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = {{ type = "copy", mode = {} }}
"#,
        toml_mode
    ));

    let error = config::read(
        &scenario.source,
        Some(scenario.target.clone()),
        scenario.env.env(),
        false,
    )
    .unwrap_err();

    assert_error_chain(&error, "invalid mode");
}

#[test]
fn a_rule_cannot_mix_symlink_with_a_mode() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "symlink", mode = "600" }
"#
    });

    let error = config::read(
        &scenario.source,
        Some(scenario.target.clone()),
        scenario.env.env(),
        false,
    )
    .unwrap_err();

    assert_error_chain(&error, "`mode` cannot be used with `type` `symlink`");
}

fn make_fifo(path: &std::path::Path) {
    use std::ffi::CString;

    let name = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
}

#[test]
fn a_literal_directory_portal_rejects_a_special_child() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        fs::create_dir(source.join("dir1")).unwrap();
        make_fifo(&source.join("dir1/fifo1"));
        r#"
[portal]
"dir1" = "dir1"
"#
    });

    let error = config::read(
        &scenario.source,
        Some(scenario.target.clone()),
        scenario.env.env(),
        false,
    )
    .unwrap_err();

    assert_error_chain(
        &error,
        format!(
            "source path `{}` is not a regular file",
            scenario.source.join("dir1/fifo1").display()
        )
        .as_str(),
    );
}

#[test]
fn a_wildcard_portal_rejects_a_special_match() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        make_fifo(&source.join("fifo2"));
        r#"
[portal]
"*" = "target1"
"#
    });

    let error = config::read(
        &scenario.source,
        Some(scenario.target.clone()),
        scenario.env.env(),
        false,
    )
    .unwrap_err();

    assert_error_chain(
        &error,
        format!(
            "source path `{}` is not a regular file",
            scenario.source.join("fifo2").display()
        )
        .as_str(),
    );
}

#[test]
fn a_literal_portal_rejects_a_special_source() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        make_fifo(&source.join("fifo1"));
        r#"
[portal]
"fifo1" = "fifo1"
"#
    });

    let error = config::read(
        &scenario.source,
        Some(scenario.target.clone()),
        scenario.env.env(),
        false,
    )
    .unwrap_err();

    assert_error_chain(
        &error,
        format!(
            "source path `{}` is not a regular file",
            scenario.source.join("fifo1").display()
        )
        .as_str(),
    );
}
