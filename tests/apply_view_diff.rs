mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;

use common::TestEnv;
use dotrift::commands::apply::{ObstructionChoice, set_prompt_choices};
use insta::assert_snapshot;
use test_case::test_case;

struct PagerEnv {
    dotrift_pager: Option<String>,
    pager: Option<String>,
}

impl PagerEnv {
    fn set(dotrift_pager: Option<&str>, pager: Option<&str>) -> Self {
        let previous = Self {
            dotrift_pager: std::env::var("DOTRIFT_PAGER").ok(),
            pager: std::env::var("PAGER").ok(),
        };
        unsafe {
            match dotrift_pager {
                Some(value) => std::env::set_var("DOTRIFT_PAGER", value),
                None => std::env::remove_var("DOTRIFT_PAGER"),
            }
            match pager {
                Some(value) => std::env::set_var("PAGER", value),
                None => std::env::remove_var("PAGER"),
            }
        }
        previous
    }
}

impl Drop for PagerEnv {
    fn drop(&mut self) {
        unsafe {
            match &self.dotrift_pager {
                Some(value) => std::env::set_var("DOTRIFT_PAGER", value),
                None => std::env::remove_var("DOTRIFT_PAGER"),
            }
            match &self.pager {
                Some(value) => std::env::set_var("PAGER", value),
                None => std::env::remove_var("PAGER"),
            }
        }
    }
}

fn write_capture_script(env: &TestEnv) -> (PathBuf, PathBuf) {
    let output = env.path("viewdiff.txt");
    let script = env.path("capture-pager.sh");
    fs::write(&script, format!("#!/bin/sh\ncat > {}\n", output.display())).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    (script, output)
}

fn install_capture_pager(env: &TestEnv) -> (PagerEnv, PathBuf) {
    let (script, output) = write_capture_script(env);
    let guard = PagerEnv::set(Some(script.to_str().unwrap()), Some(""));
    (guard, output)
}

#[test_case(
    |source: &Path, target: &Path| {
        fs::write(source.join("file.txt"), b"new content\n").unwrap();
        fs::write(target.join("target.txt"), b"old content\n").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"copy\" }\n"
    }
    ; "copy_changes_single_line"
)]
#[test_case(
    |source: &Path, target: &Path| {
        fs::write(source.join("dotrift_data.toml"), "[variable]\nmessage = \"hello\"\n").unwrap();
        fs::write(source.join("greeting.txt"), "{{ message }}\n").unwrap();
        fs::write(target.join("target.txt"), b"old\n").unwrap();
        "[portal]\n\"greeting.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"template\" }\n"
    }
    ; "template_renders_into_diff"
)]
fn view_diff_snapshot<F: Fn(&Path, &Path) -> &'static str>(setup: F) {
    let env = TestEnv::new();
    let source = env.path("source");
    let target = env.path("target");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&target).unwrap();
    let config = setup(&source, &target);
    fs::write(source.join("dotrift.toml"), config).unwrap();
    let (_pager, diff_file) = install_capture_pager(&env);
    set_prompt_choices([ObstructionChoice::ViewDiff, ObstructionChoice::Skip]);
    dotrift::commands::apply::run(&source, Some(target.clone())).unwrap();
    let diff = fs::read_to_string(diff_file).unwrap();
    snapshot_with_root_filter(&env, &diff);
}

fn write_copy_diff_fixture(env: &TestEnv) -> (PathBuf, PathBuf) {
    let source = env.path("source");
    let target = env.path("target");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&target).unwrap();
    fs::write(source.join("file.txt"), b"new content\n").unwrap();
    fs::write(target.join("target.txt"), b"old content\n").unwrap();
    fs::write(
        source.join("dotrift.toml"),
        "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"copy\" }\n",
    )
    .unwrap();
    (source, target)
}

fn snapshot_with_root_filter(env: &TestEnv, output: &str) {
    let test_name = std::thread::current().name().unwrap().replace(":", "_");
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(env.root().to_str().unwrap(), "<root>");
    settings.bind(|| {
        assert_snapshot!(test_name, output);
    });
}

fn run_view_diff_with_env(env: &TestEnv, dotrift_pager: Option<&str>, pager: Option<&str>) {
    let (source, target) = write_copy_diff_fixture(env);
    let _guard = PagerEnv::set(dotrift_pager, pager);
    set_prompt_choices([ObstructionChoice::ViewDiff, ObstructionChoice::Skip]);
    dotrift::capture::clear();
    dotrift::commands::apply::run(&source, Some(target)).unwrap();
}

#[test]
fn dotrift_pager_blank_defers_to_pager() {
    let env = TestEnv::new();
    let (script, output) = write_capture_script(&env);
    run_view_diff_with_env(&env, Some(""), Some(script.to_str().unwrap()));
    let diff = fs::read_to_string(output).unwrap();
    snapshot_with_root_filter(&env, &diff);
}

#[test]
fn dotrift_pager_whitespace_defers_to_pager() {
    let env = TestEnv::new();
    let (script, output) = write_capture_script(&env);
    run_view_diff_with_env(&env, Some("   "), Some(script.to_str().unwrap()));
    let diff = fs::read_to_string(output).unwrap();
    snapshot_with_root_filter(&env, &diff);
}

#[test]
fn pager_env_unset_prints_to_stdout() {
    let env = TestEnv::new();
    run_view_diff_with_env(&env, None, None);
    let captured = dotrift::capture::take();
    snapshot_with_root_filter(&env, &captured);
}

#[test]
fn pager_env_falls_back_to_stdout_on_pager_failure() {
    let env = TestEnv::new();
    let missing = env.path("no-such-pager");
    run_view_diff_with_env(&env, Some(""), Some(missing.to_str().unwrap()));
    let captured = dotrift::capture::take();
    snapshot_with_root_filter(&env, &captured);
}

#[test]
fn dotrift_pager_failure_is_an_error() {
    let env = TestEnv::new();
    let missing = env.path("no-such-pager");
    let (source, target) = write_copy_diff_fixture(&env);
    let _guard = PagerEnv::set(Some(missing.to_str().unwrap()), Some(""));
    set_prompt_choices([ObstructionChoice::ViewDiff, ObstructionChoice::Skip]);
    let error = dotrift::commands::apply::run(&source, Some(target)).unwrap_err();
    let rendered = format!("{error}");
    assert!(rendered.contains("cannot run DOTRIFT_PAGER"), "{rendered}");
}
