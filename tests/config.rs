mod common;

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::PathBuf,
};

use dotrift::{
    config::{self, DeploymentEntry, DesiredDeployment},
    deploy_entry,
};
use templater::value::Value;
use test_case::test_case;

use common::{EnvVarGuard, TestEnv, assert_error_chain};

#[test]
fn read_assembles_desired_deployment_from_config_and_data() {
    let env = TestEnv::new();
    let source = env.source_dir();
    let target = env.target_dir();
    fs::write(source.join("vimrc"), b"set number").unwrap();
    fs::create_dir_all(source.join("nvim/lua")).unwrap();
    fs::write(source.join("nvim/init.lua"), b"-- init").unwrap();
    fs::write(source.join("nvim/lua/mappings.lua"), b"-- mappings").unwrap();
    env.write_data_file("[variable]\neditor = \"nvim\"\n");
    env.write_config(
        "[portal]\n\
         \"vimrc\" = \".vimrc\"\n\
         \"nvim\" = \".config/nvim\"\n\
         \n\
         [rule]\n\
         \".vimrc\" = { type = \"copy\", mode = \"644\" }\n\
         \".config/nvim/init.lua\" = { type = \"template\" }\n",
    );

    let mut deployment =
        config::read(&source, Some(target.clone())).expect("cannot read configuration");

    let mut entries = vec![
        deploy_entry!(
            source.join("nvim/init.lua"),
            target.join(".config/nvim/init.lua"),
            Template
        ),
        deploy_entry!(
            source.join("nvim/lua/mappings.lua"),
            target.join(".config/nvim/lua/mappings.lua"),
            Symlink
        ),
        deploy_entry!(source.join("vimrc"), target.join(".vimrc"), Copy, 0o644),
    ];
    entries.sort_by(|left, right| left.target_path.cmp(&right.target_path));
    deployment
        .entries
        .sort_by(|left, right| left.target_path.cmp(&right.target_path));
    assert_eq!(
        deployment,
        DesiredDeployment {
            target_directory: target,
            entries,
            variable_context: HashMap::from([("editor".to_string(), Value::Str("nvim".into()))]),
        }
    );
}

#[test]
fn active_profile_overrides_base_variable_in_rendered_config_and_context() {
    let env = TestEnv::new();
    let source = env.source_dir();
    let target = env.target_dir();
    fs::write(source.join("vimrc"), b"set number").unwrap();
    env.write_data_file("[variable]\nhost = \"base\"\n\n[profile.work]\nhost = \"work\"\n");
    env.database().activate_profile("work").unwrap();
    env.write_config("[portal]\n\"vimrc\" = \".config/{{ host }}/vimrc\"\n");

    let deployment =
        config::read(&source, Some(target.clone())).expect("cannot read configuration");

    assert_eq!(
        deployment,
        DesiredDeployment {
            target_directory: target.clone(),
            entries: vec![deploy_entry!(
                source.join("vimrc"),
                target.join(".config/work/vimrc"),
                Symlink
            )],
            variable_context: HashMap::from([("host".to_string(), Value::Str("work".into()))]),
        }
    );
}

#[test]
fn active_profile_without_definition_is_ignored() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file("[variable]\nhost = \"base\"\n");
    env.database().activate_profile("gone").unwrap();
    env.write_config("");

    let deployment =
        config::read(&source, Some(env.target_dir())).expect("cannot read configuration");

    assert_eq!(
        deployment.variable_context,
        HashMap::from([("host".to_string(), Value::Str("base".into()))])
    );
}

#[test]
fn missing_template_variable_fails_before_parsing() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_config("[portal]\n\"vimrc\" = \"{{ absent }}/vimrc\"\n");

    let error =
        config::read(&source, Some(env.target_dir())).expect_err("undefined variable must fail");

    assert_error_chain(&error, "absent");
    assert!(
        !error
            .chain()
            .any(|cause| cause.to_string().contains("cannot parse")),
        "template errors must precede TOML parsing but got: {error:?}"
    );
}

#[test_case(
    |env| {
        env.write_data_file("[variable]\nhost = \"../evil\"\n");
        env.write_config("[portal]\n\"vimrc\" = \"{{ host }}/vimrc\"\n");
    },
    &["invalid portal target path"] ;
    "rendered_values_are_validated_like_literals"
)]
#[test_case(
    |env| {
        env.write_config("");
        fs::create_dir(env.source_dir().join(".dotriftignore")).unwrap();
    },
    &[".dotriftignore"] ;
    "unreadable_dotriftignore_fails_as_configuration_error"
)]
#[test_case(
    |env| {
        env.write_config("");
        env.write_data_file("[variable\nbroken\n");
    },
    &["dotrift_data.toml"] ;
    "malformed_data_file_names_path_in_error_chain"
)]
#[test_case(
    |env| {
        env.write_config("");
        env.write_data_file("[variable]\nratio = 2.5\n");
    },
    &["dotrift_data.toml"] ;
    "float_variable_value_is_a_configuration_error"
)]
#[test_case(
    |_env| {},
    &["dotrift.toml"] ;
    "missing_dotrift_toml_names_path_in_chain"
)]
#[test_case(
    |env| env.write_config("= broken =\n"),
    &["cannot parse", "dotrift.toml"] ;
    "malformed_toml_after_render_names_config_in_chain"
)]
#[test_case(
    |env| env.write_config("version = \"2\"\n"),
    &[r#"unknown field `version`"#] ;
    "unknown_root_key_is_rejected"
)]
#[test_case(
    |env| {
        fs::write(env.source_dir().join("a.txt"), b"a").unwrap();
        fs::write(env.source_dir().join("b.txt"), b"b").unwrap();
        env.write_config("[portal]\n\"a.txt\" = \".shared\"\n\"b.txt\" = \".shared\"\n");
    },
    &["collision at"] ;
    "collision_across_portals_reports_collision_at"
)]
fn read_fails_with_expected_error(setup: impl FnOnce(&TestEnv), expected: &[&str]) {
    let env = TestEnv::new();
    setup(&env);

    let error = config::read(&env.source_dir(), Some(env.target_dir()))
        .expect_err("configuration must fail");

    for fragment in expected {
        assert_error_chain(&error, fragment);
    }
}

#[test]
fn relative_target_directory_is_rejected() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_config("target-directory = \"relative/path\"\n");

    let error =
        config::read(&source, None).expect_err("relative target-directory must be rejected");

    assert_error_chain(&error, "must be an absolute path");
}

#[test_case(true ; "override_beats_configured_target_directory")]
#[test_case(false ; "configured_target_directory_used_without_override")]
fn configured_target_directory_resolves(use_override: bool) {
    let env = TestEnv::new();
    let configured = env.path("configured-target");
    env.write_config(&format!(
        "target-directory = \"{}\"\n",
        configured.display()
    ));

    let deployment = config::read(&env.source_dir(), use_override.then(|| env.target_dir()))
        .expect("cannot read configuration");

    assert_eq!(
        deployment.target_directory,
        if use_override {
            env.target_dir()
        } else {
            configured
        }
    );
}

#[test]
fn home_fallback_when_no_override_and_no_configured_target() {
    let env = TestEnv::new();
    let source = env.source_dir();
    let home = env.path("home");
    let _guard = EnvVarGuard::set([("HOME", Some(home.to_str().unwrap()))]);
    env.write_config("");
    let deployment = config::read(&source, None).expect("cannot read configuration");
    assert_eq!(deployment.target_directory, home);
}

#[test_case(
    |env| {
        fs::write(env.source_dir().join("vimrc"), b"set number").unwrap();
        fs::create_dir_all(env.source_dir().join("notes")).unwrap();
        fs::write(env.source_dir().join("notes/todo.txt"), b"ship").unwrap();
        env.write_data_file("");
        env.write_config("[portal]\n\"**\" = \".\"\n");
        fs::write(env.source_dir().join(".dotriftignore"), "vimrc\n").unwrap();
        vec![deploy_entry!(
            env.source_dir().join("notes/todo.txt"),
            env.target_dir().join("notes/todo.txt"),
            Symlink
        )]
    } ;
    "dotriftignore_excludes_entry_and_control_files_never_deploy"
)]
#[test_case(
    |env| {
        fs::write(env.source_dir().join("keep.conf"), b"keep").unwrap();
        fs::write(env.source_dir().join("skip.conf"), b"skip").unwrap();
        env.write_config("[portal]\n\"*.conf\" = \".conf\"\n");
        fs::write(env.source_dir().join(".dotriftignore"), "*.conf\n!keep.conf\n").unwrap();
        vec![deploy_entry!(
            env.source_dir().join("keep.conf"),
            env.target_dir().join(".conf/keep.conf"),
            Symlink
        )]
    } ;
    "negation_reincludes_target_ignored_by_earlier_pattern"
)]
#[test_case(
    |env| {
        env.write_data_file("");
        env.write_config("[portal]\n\"**\" = \".\"\n");
        fs::write(env.source_dir().join(".dotriftignore"), "!dotrift.toml\n").unwrap();
        vec![deploy_entry!(
            env.source_dir().join("dotrift.toml"),
            env.target_dir().join("dotrift.toml"),
            Symlink
        )]
    } ;
    "negated_pattern_reincludes_control_file"
)]
#[test_case(
    |env| {
        fs::create_dir_all(env.source_dir().join("sub")).unwrap();
        fs::write(env.source_dir().join("sub/dotrift.toml"), b"# nested").unwrap();
        env.write_config("[portal]\n\"sub/dotrift.toml\" = \".config/sub/dotrift.toml\"\n");
        vec![deploy_entry!(
            env.source_dir().join("sub/dotrift.toml"),
            env.target_dir().join(".config/sub/dotrift.toml"),
            Symlink
        )]
    } ;
    "nested_control_filename_deploys_normally"
)]
#[test_case(
    |env| {
        fs::write(env.source_dir().join("a.txt"), b"a").unwrap();
        fs::write(env.source_dir().join("b.txt"), b"b").unwrap();
        env.write_config("[portal]\n\"a.txt\" = \".shared\"\n\"b.txt\" = \".shared\"\n");
        fs::write(env.source_dir().join(".dotriftignore"), "/.shared\n").unwrap();
        vec![]
    } ;
    "ignoring_colliding_targets_prevents_collision_validation"
)]
fn portal_mapping_yields_expected_entries(setup: impl FnOnce(&TestEnv) -> Vec<DeploymentEntry>) {
    let env = TestEnv::new();
    let expected = setup(&env);

    let deployment =
        config::read(&env.source_dir(), Some(env.target_dir())).expect("cannot read configuration");

    assert_eq!(deployment.entries, expected);
}

#[test]
fn profile_overlay_replaces_whole_value_without_recursive_merge() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_data_file(
        "[variable]\nsettings = { a = 1, b = 2 }\n\n[profile.work]\nsettings = { c = 3 }\n",
    );
    env.database().activate_profile("work").unwrap();
    env.write_config("");

    let deployment =
        config::read(&source, Some(env.target_dir())).expect("cannot read configuration");

    assert_eq!(
        deployment.variable_context,
        HashMap::from([(
            "settings".to_string(),
            Value::Map(BTreeMap::from([("c".to_string(), Value::Int(3))]))
        )])
    );
}

#[test_case(
    |env| env.root().join("missing") ;
    "missing_source_dir_is_rejected"
)]
#[test_case(
    |env| {
        let file = env.root().join("plain-file");
        fs::write(&file, b"data").unwrap();
        file
    } ;
    "source_path_that_is_a_file_is_rejected"
)]
fn source_that_is_not_a_directory_fails_with_does_not_exist(
    setup: impl FnOnce(&TestEnv) -> PathBuf,
) {
    let env = TestEnv::new();
    let source = setup(&env);
    let error =
        config::read(&source, Some(env.target_dir())).expect_err("non-directory source must fail");
    assert_error_chain(&error, "does not exist");
}

#[test]
fn target_inside_source_reports_overlap() {
    let env = TestEnv::new();
    let source = env.source_dir();
    env.write_config("");

    let error = config::read(&source, Some(source.join("inside")))
        .expect_err("target inside source must fail");

    assert_error_chain(&error, "source and target directories overlap");
}
