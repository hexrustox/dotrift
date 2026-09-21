mod common;

use std::fs;

use common::{ApplyScenario, Prompt, TestEnv, assert_error_chain, record_of};
use dotrift::commands::apply::ApplyOptions;
use dotrift::deploy::ObstructionChoice;
use dotrift::state::{Kind, TemplateHash, hash_bytes};

fn registry_is_clean(env: &TestEnv) -> bool {
    let dir = env.path("render-registry/registry");
    match fs::symlink_metadata(&dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Ok(metadata) if metadata.file_type().is_dir() => {
            fs::read_dir(&dir).unwrap().next().is_none()
        }
        _ => false,
    }
}

fn block_registry(env: &TestEnv) {
    let registry = env.path("render-registry/registry");
    fs::create_dir_all(registry.parent().unwrap()).unwrap();
    fs::write(&registry, "content1").unwrap();
}

#[test]
fn multi_target_template_shares_one_render_and_cleans_registry() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "{{ str }}\n").unwrap();
        fs::write(source.join("file2"), "{{ str }}\n").unwrap();
        r#"
[portal]
"file1" = "file1"
"file2" = "file2"

[rule]
"file1" = { type = "template" }
"file2" = { type = "template" }
"#
    });
    scenario.env.write_data_file("[variable]\nstr = \"str\"\n");

    scenario.run();

    for name in ["file1", "file2"] {
        let target = scenario.target.join(name);
        assert_eq!(fs::read(&target).unwrap(), b"str\n");
        let record =
            record_of(&scenario.env, &target).expect("no state record for deployed template");
        assert_eq!(record.kind, Kind::File);
        assert_eq!(
            record.content_hash.as_deref(),
            Some(hash_bytes(b"str\n").as_str())
        );
    }
    assert!(
        registry_is_clean(&scenario.env),
        "render registry was not cleaned after a shared multi-target render"
    );
}

#[test]
fn render_failure_leaves_target_absent_and_empties_registry() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "{{ str").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "template" }
"#
    });
    scenario.env.write_data_file("[variable]\nstr = \"str\"\n");

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, "cannot render");
    assert!(
        !scenario.target.join("file1").exists(),
        "failed render must leave the target absent"
    );
    assert!(
        record_of(&scenario.env, &scenario.target.join("file1")).is_none(),
        "failed render must record nothing"
    );
    assert!(
        registry_is_clean(&scenario.env),
        "render registry was not emptied after a render failure"
    );
}

#[test]
fn stale_registry_entry_from_killed_run_is_discarded_and_rerendered() {
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

    let stale_name = format!("{}.tmpl", TemplateHash::of_bytes(b"{{ str }}\n"));
    let registry = scenario.env.path("render-registry/registry");
    fs::create_dir_all(&registry).unwrap();
    fs::write(registry.join(&stale_name), "content1").unwrap();

    scenario.run();

    let target = scenario.target.join("file1");
    assert_eq!(fs::read(&target).unwrap(), b"str\n");
    let record = record_of(&scenario.env, &target).expect("no state record after re-render");
    assert_eq!(
        record.content_hash.as_deref(),
        Some(hash_bytes(b"str\n").as_str())
    );
    assert!(
        registry_is_clean(&scenario.env),
        "stale registry entry was not discarded and cleaned"
    );
}

#[test]
fn dry_run_creates_nothing_in_registry() {
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

    scenario.run_with_options(ApplyOptions {
        dry_run: true,
        ..Default::default()
    });

    assert!(
        !scenario.env.path("render-registry/registry").exists(),
        "dry run must create nothing in the render registry"
    );
    assert!(
        !scenario.target.join("file1").exists(),
        "dry run must deploy nothing"
    );
}

#[test]
fn dry_run_replaces_preexisting_registry_and_leaves_nothing() {
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

    let registry = scenario.env.path("render-registry/registry");
    fs::create_dir_all(&registry).unwrap();
    fs::write(registry.join("file1"), "content1").unwrap();

    scenario.run_with_options(ApplyOptions {
        dry_run: true,
        ..Default::default()
    });

    assert!(
        !registry.join("file1").exists(),
        "dry run must empty a preexisting registry like a real run"
    );
    assert!(
        !scenario.target.join("file1").exists(),
        "dry run must deploy nothing"
    );
}

#[test]
fn view_diff_fails_when_registry_unavailable_leaving_target_intact() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::write(source.join("file1"), "{{ str }}\n").unwrap();
        fs::write(target.join("file1"), "content1\n").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "template" }
"#
    });
    scenario.env.write_data_file("[variable]\nstr = \"str\"\n");
    block_registry(&scenario.env);

    let error = scenario
        .try_run_with_prompter(&Prompt::once(ObstructionChoice::ViewDiff))
        .unwrap_err();

    assert_error_chain(&error, "template render registry is unavailable");
    assert_eq!(
        fs::read(scenario.target.join("file1")).unwrap(),
        b"content1\n",
        "failed view diff must leave the target intact"
    );
    assert!(
        record_of(&scenario.env, &scenario.target.join("file1")).is_none(),
        "failed view diff must record nothing"
    );
}

#[test]
fn template_deploy_falls_back_to_direct_render_when_registry_unavailable() {
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
    block_registry(&scenario.env);

    scenario.run();

    let target = scenario.target.join("file1");
    assert_eq!(fs::read(&target).unwrap(), b"str\n");
    let record =
        record_of(&scenario.env, &target).expect("no state record for direct-rendered template");
    assert_eq!(record.kind, Kind::File);
    assert_eq!(
        record.content_hash.as_deref(),
        Some(hash_bytes(b"str\n").as_str())
    );
    assert!(
        fs::symlink_metadata(scenario.env.path("render-registry/registry"))
            .unwrap()
            .file_type()
            .is_file(),
        "registry must stay unavailable when direct rendering falls back"
    );
}

#[test]
fn view_diff_then_replace_deploys_shared_render_and_cleans_registry() {
    let scenario = ApplyScenario::new(|source, target| {
        fs::write(source.join("file1"), "{{ str }}\n").unwrap();
        fs::write(target.join("file1"), "content1\n").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "template" }
"#
    });
    scenario.env.write_data_file("[variable]\nstr = \"str\"\n");

    let _guard = scenario
        .env
        .set_vars([("DOTRIFT_PAGER", None), ("PAGER", None)]);
    dotrift::report::clear();
    scenario.run_with_prompter(&Prompt::sequence([
        ObstructionChoice::ViewDiff,
        ObstructionChoice::Replace,
    ]));

    let target = scenario.target.join("file1");
    assert_eq!(fs::read(&target).unwrap(), b"str\n");
    let record =
        record_of(&scenario.env, &target).expect("no state record after view-diff replace");
    assert_eq!(record.kind, Kind::File);
    assert_eq!(
        record.content_hash.as_deref(),
        Some(hash_bytes(b"str\n").as_str())
    );
    assert!(
        registry_is_clean(&scenario.env),
        "render registry was not cleaned after view-diff then replace"
    );
    let diff = dotrift::report::take_output();
    assert!(
        diff.contains("-content1") && diff.contains("+str"),
        "view diff must show the rendered template diff, got: {diff}"
    );
}
