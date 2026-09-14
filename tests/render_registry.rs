mod common;

use std::{fs, path::Path, path::PathBuf};

use common::{ApplyScenario, Prompt, TestEnv, assert_error_chain, record_of};
use dotrift::commands::apply::ApplyOptions;
use dotrift::deploy::ObstructionChoice;
use dotrift::state::hash_bytes;

fn registry_dir(env: &TestEnv) -> PathBuf {
    env.path("render-registry/registry")
}

/// One template deployed to a fresh target path.
fn template_setup(source: &Path, _target: &Path) -> &'static str {
    fs::write(
        source.join("dotrift_data.toml"),
        "[variable]\ngreeting = \"hello\"\n",
    )
    .unwrap();
    fs::write(source.join("greeting.txt"), "{{ greeting }}\n").unwrap();
    "[portal]\n\"greeting.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"template\" }\n"
}

/// The same, but with an untracked obstruction already at the target path.
fn obstructed_template_setup(source: &Path, target: &Path) -> &'static str {
    fs::write(target.join("target.txt"), b"old\n").unwrap();
    template_setup(source, target)
}

#[test]
fn multi_target_template_deploys_the_same_render_to_every_target() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(
            source.join("dotrift_data.toml"),
            "[variable]\ngreeting = \"hello\"\n",
        )
        .unwrap();
        fs::create_dir_all(source.join("dir")).unwrap();
        fs::write(source.join("dir/greeting.txt"), "{{ greeting }}\n").unwrap();
        "[portal]\n\"dir\" = \".\"\n\"dir/greeting.txt\" = \"alias.txt\"\n[rule]\n\"alias.txt\" = { type = \"template\" }\n\"greeting.txt\" = { type = \"template\" }\n"
    });
    scenario.run();

    for name in ["alias.txt", "greeting.txt"] {
        let target = scenario.target.join(name);
        assert_eq!(fs::read(&target).unwrap(), b"hello\n");
        let record = record_of(&scenario.env, &target).expect("state record missing");
        assert_eq!(
            record.content_hash.as_deref(),
            Some(hash_bytes(b"hello\n").as_str())
        );
    }
    assert!(!registry_dir(&scenario.env).exists());
}

#[test]
fn render_failure_leaves_the_target_absent_and_the_registry_emptied() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("greeting.txt"), "{{ absent }}\n").unwrap();
        "[portal]\n\"greeting.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"template\" }\n"
    });

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot render");
    assert!(!scenario.target.join("target.txt").exists());
    assert!(!registry_dir(&scenario.env).exists());
}

#[test]
fn stale_entry_from_a_killed_run_is_discarded() {
    let scenario = ApplyScenario::new(template_setup);
    let registry = registry_dir(&scenario.env);
    fs::create_dir_all(&registry).unwrap();
    let template_bytes = fs::read(scenario.source.join("greeting.txt")).unwrap();
    let template_hash = hash_bytes(&template_bytes);
    let stale = registry.join(format!("{template_hash}.tmpl"));
    fs::write(&stale, b"STALE\n").unwrap();

    scenario.run();

    assert_eq!(
        fs::read(scenario.target.join("target.txt")).unwrap(),
        b"hello\n"
    );
    assert!(!stale.exists());
}

#[test]
fn dry_run_creates_nothing_in_the_registry() {
    let scenario = ApplyScenario::new(template_setup);
    scenario.run_with_options(ApplyOptions {
        dry_run: true,
        ..Default::default()
    });

    assert!(!scenario.env.path("render-registry").exists());
}

#[test]
fn dry_run_leaves_a_preexisting_registry_untouched() {
    let scenario = ApplyScenario::new(template_setup);
    let stale = registry_dir(&scenario.env).join("stale.tmpl");
    fs::create_dir_all(registry_dir(&scenario.env)).unwrap();
    fs::write(&stale, b"STALE\n").unwrap();

    scenario.run_with_options(ApplyOptions {
        dry_run: true,
        ..Default::default()
    });

    assert_eq!(fs::read(&stale).unwrap(), b"STALE\n");
}

#[test]
fn view_diff_fails_when_the_registry_is_unavailable() {
    let scenario = ApplyScenario::new(obstructed_template_setup);
    fs::write(scenario.env.path("render-registry"), b"").unwrap();
    let prompter = Prompt::sequence([ObstructionChoice::ViewDiff, ObstructionChoice::Skip]);

    let error = scenario.try_run_with_prompter(&prompter).unwrap_err();

    assert_error_chain(&error, "template render registry is unavailable");
    assert_eq!(
        fs::read(scenario.target.join("target.txt")).unwrap(),
        b"old\n"
    );
}

#[test]
fn template_deploy_falls_back_when_the_registry_is_unavailable() {
    let scenario = ApplyScenario::new(template_setup);
    fs::write(scenario.env.path("render-registry"), b"").unwrap();

    scenario.run();

    assert_eq!(
        fs::read(scenario.target.join("target.txt")).unwrap(),
        b"hello\n"
    );
    let record = record_of(&scenario.env, &scenario.target.join("target.txt"))
        .expect("state record missing");
    assert_eq!(
        record.content_hash.as_deref(),
        Some(hash_bytes(b"hello\n").as_str())
    );
    assert!(!registry_dir(&scenario.env).exists());
}

#[test]
fn view_diff_then_replace_deploys_the_shared_render() {
    let scenario = ApplyScenario::new(obstructed_template_setup);
    let prompter = Prompt::sequence([ObstructionChoice::ViewDiff, ObstructionChoice::Replace]);

    scenario.run_with_prompter(&prompter);

    assert_eq!(
        fs::read(scenario.target.join("target.txt")).unwrap(),
        b"hello\n"
    );
    let record = record_of(&scenario.env, &scenario.target.join("target.txt"))
        .expect("state record missing");
    assert_eq!(
        record.content_hash.as_deref(),
        Some(hash_bytes(b"hello\n").as_str())
    );
    assert!(!registry_dir(&scenario.env).exists());
}
