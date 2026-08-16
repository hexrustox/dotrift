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
    fn set(dotrift_pager: &str, pager: &str) -> Self {
        let previous = Self {
            dotrift_pager: std::env::var("DOTRIFT_PAGER").ok(),
            pager: std::env::var("PAGER").ok(),
        };
        unsafe {
            std::env::set_var("DOTRIFT_PAGER", dotrift_pager);
            std::env::set_var("PAGER", pager);
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

fn install_capture_pager(env: &TestEnv) -> (PagerEnv, PathBuf) {
    let output = env.path("viewdiff.txt");
    let script = env.path("capture-pager.sh");
    fs::write(&script, format!("#!/bin/sh\ncat > {}\n", output.display())).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let guard = PagerEnv::set(script.to_str().unwrap(), "");
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
    let test_name = std::thread::current().name().unwrap().replace(":", "_");
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(env.root().to_str().unwrap(), "<root>");
    settings.bind(|| {
        assert_snapshot!(test_name, diff);
    });
}
