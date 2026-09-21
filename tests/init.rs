mod common;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use common::{TestEnv, assert_error_chain, snapshot_settings, test_name};
use test_case::test_case;

fn init_run(env: &TestEnv, source: &Path) -> (Result<(), miette::Report>, String) {
    dotrift::report::clear();
    let result = dotrift::commands::init::run(source, env.env(), false);
    let output = dotrift::report::take_output();
    (result, output)
}

fn control_file_names() -> [&'static str; 3] {
    ["dotrift.toml", "dotrift_data.toml", ".dotriftignore"]
}

#[test]
fn init_creates_a_missing_source_directory_with_parents_and_all_control_files() {
    let env = TestEnv::new();
    let source = env.path("dir1/sub1/source");

    let (result, output) = init_run(&env, &source);

    result.expect("init failed");
    assert!(source.is_dir(), "expected the source directory to exist");
    for name in control_file_names() {
        assert!(
            source.join(name).is_file(),
            "expected `{name}` to exist:\n{output}"
        );
    }
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 3, "expected one line per file:\n{output}");
    for (line, name) in lines.iter().zip(control_file_names()) {
        assert_eq!(
            line,
            &source.join(name).to_str().unwrap(),
            "expected the created path for `{name}`:\n{output}"
        );
    }
}

#[test_case("dotrift.toml"; "dotrift_toml")]
#[test_case("dotrift_data.toml"; "data_file")]
#[test_case(".dotriftignore"; "ignore_file")]
fn init_scaffold_matches_the_spec(name: &str) {
    let env = TestEnv::new();
    let source = env.source_dir();

    let (result, _) = init_run(&env, &source);
    result.expect("init failed");

    let contents = fs::read_to_string(source.join(name)).unwrap();
    snapshot_settings(&env).bind(|| {
        insta::assert_snapshot!(test_name(), contents);
    });
}

#[test]
fn init_fills_an_existing_empty_directory() {
    let env = TestEnv::new();
    let source = env.source_dir();

    let (result, _) = init_run(&env, &source);

    result.expect("init failed");
    for name in control_file_names() {
        assert!(source.join(name).is_file(), "expected `{name}` to exist");
    }
}

#[test]
fn init_refuses_an_already_initialized_directory() {
    let env = TestEnv::new();
    let source = env.source_dir();
    fs::write(source.join("dotrift.toml"), "content1").unwrap();

    let (result, output) = init_run(&env, &source);

    assert_error_chain(
        &result.expect_err("init should refuse"),
        "already initialized",
    );
    assert_eq!(
        fs::read_to_string(source.join("dotrift.toml")).unwrap(),
        "content1"
    );
    assert!(!source.join("dotrift_data.toml").exists());
    assert!(!source.join(".dotriftignore").exists());
    assert_eq!(output, "", "a refused run prints nothing to stdout");
}

#[test]
fn init_preserves_pre_existing_optional_control_files() {
    let env = TestEnv::new();
    let source = env.source_dir();
    fs::write(source.join("dotrift_data.toml"), "content1").unwrap();
    fs::write(source.join(".dotriftignore"), "content2").unwrap();

    let (result, output) = init_run(&env, &source);

    result.expect("init failed");
    assert_eq!(
        fs::read_to_string(source.join("dotrift_data.toml")).unwrap(),
        "content1"
    );
    assert_eq!(
        fs::read_to_string(source.join(".dotriftignore")).unwrap(),
        "content2"
    );
    assert!(source.join("dotrift.toml").is_file());
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(
        lines,
        vec![source.join("dotrift.toml").to_str().unwrap()],
        "only the created file is listed:\n{output}"
    );
}

#[test]
fn init_treats_a_dangling_symlink_at_dotrift_toml_as_present() {
    let env = TestEnv::new();
    let source = env.source_dir();
    symlink("file1", source.join("dotrift.toml")).unwrap();

    let (result, output) = init_run(&env, &source);

    assert_error_chain(
        &result.expect_err("init should refuse"),
        "already initialized",
    );
    assert!(
        fs::symlink_metadata(source.join("dotrift.toml"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "expected the symlink to be preserved"
    );
    assert!(
        !source.join("file1").exists(),
        "must not write through the symlink"
    );
    assert_eq!(output, "");
}

#[test]
fn init_treats_dangling_symlinks_at_optional_paths_as_present() {
    let env = TestEnv::new();
    let source = env.source_dir();
    symlink("file1", source.join("dotrift_data.toml")).unwrap();
    symlink("file2", source.join(".dotriftignore")).unwrap();

    let (result, output) = init_run(&env, &source);

    result.expect("init failed");
    assert!(source.join("dotrift.toml").is_file());
    for (name, reference) in [("dotrift_data.toml", "file1"), (".dotriftignore", "file2")] {
        assert!(
            fs::symlink_metadata(source.join(name))
                .unwrap()
                .file_type()
                .is_symlink(),
            "expected the symlink at `{name}` to be preserved"
        );
        assert!(
            !source.join(reference).exists(),
            "must not write through `{name}`"
        );
    }
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(
        lines,
        vec![source.join("dotrift.toml").to_str().unwrap()],
        "only the created file is listed:\n{output}"
    );
}

#[test]
fn init_errors_when_the_source_path_is_a_file() {
    let env = TestEnv::new();
    let source = env.path("file1");
    fs::write(&source, "content1").unwrap();

    let (result, output) = init_run(&env, &source);

    assert_error_chain(&result.expect_err("init should fail"), "is not a directory");
    assert_eq!(fs::read_to_string(&source).unwrap(), "content1");
    assert_eq!(output, "");
}

#[test]
fn init_errors_when_the_source_symlink_dangles() {
    let env = TestEnv::new();
    let source = env.path("link1");
    symlink("file1", &source).unwrap();

    let (result, output) = init_run(&env, &source);

    assert_error_chain(
        &result.expect_err("init should fail"),
        "does not resolve to a directory",
    );
    assert!(
        fs::symlink_metadata(&source)
            .unwrap()
            .file_type()
            .is_symlink(),
        "expected the dangling symlink to be preserved"
    );
    assert!(!env.path("file1").exists());
    assert_eq!(output, "");
}

#[test]
fn init_errors_when_the_source_symlink_resolves_to_a_file() {
    let env = TestEnv::new();
    let file = env.path("file1");
    fs::write(&file, "content1").unwrap();
    let source = env.path("link1");
    symlink("file1", &source).unwrap();

    let (result, _) = init_run(&env, &source);

    assert_error_chain(&result.expect_err("init should fail"), "is not a directory");
}

#[test]
fn init_follows_a_symlink_to_a_directory() {
    let env = TestEnv::new();
    let target = env.path("dir1");
    fs::create_dir_all(&target).unwrap();
    let source = env.path("link1");
    symlink("dir1", &source).unwrap();

    let (result, output) = init_run(&env, &source);

    result.expect("init failed");
    for name in control_file_names() {
        assert!(
            target.join(name).is_file(),
            "expected `{name}` inside the linked directory"
        );
    }
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(
        lines,
        vec![
            source.join("dotrift.toml").to_str().unwrap(),
            source.join("dotrift_data.toml").to_str().unwrap(),
            source.join(".dotriftignore").to_str().unwrap(),
        ],
        "paths display through the logical link path:\n{output}"
    );
}

#[test]
fn init_touches_no_state_database_or_global_config() {
    let env = TestEnv::new();
    let source = env.source_dir();

    let (result, _) = init_run(&env, &source);

    result.expect("init failed");
    assert!(!env.path("state").exists(), "no state directory is created");
    assert!(
        !env.path("config-home/dotrift/config.toml").exists(),
        "no global config is created"
    );
}
