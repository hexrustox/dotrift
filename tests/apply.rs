mod common;

use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};

use common::{ApplyScenario, TestEnv, assert_error_chain, prompt_count};
use dotrift::ExitStatus;
use dotrift::commands::apply::{ApplyOptions, ObstructionChoice, test_hooks::set_prompt_choice};
use dotrift::hash::hash_bytes;
use dotrift::state::{Kind, StateDatabase};
use test_case::test_case;

fn record_of(path: &Path) -> Option<dotrift::state::StateRecord> {
    StateDatabase::open()
        .expect("cannot open state database")
        .record(path)
        .unwrap()
}

#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"hello").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    |source: &Path, target: &Path| {
        let link = target.join("target.txt");
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_link(&link).unwrap(), source.join("file.txt"));
        assert_eq!(fs::read(&link).unwrap(), b"hello");
        let record = record_of(&link).unwrap();
        assert_eq!(record.kind, Kind::Symlink);
        assert_eq!(record.source_path, source.join("file.txt"));
    }
    ; "single_file_portal_deploys_symlink"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("real.txt"), b"hello").unwrap();
        symlink("real.txt", source.join("link.txt")).unwrap();
        "[portal]\n\"link.txt\" = \"target.txt\"\n"
    },
    |source: &Path, target: &Path| {
        let link = target.join("target.txt");
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_link(&link).unwrap(), source.join("link.txt"));
        assert_eq!(fs::read(&link).unwrap(), b"hello");
        let record = record_of(&link).unwrap();
        assert_eq!(record.kind, Kind::Symlink);
        assert_eq!(record.source_path, source.join("link.txt"));
    }
    ; "symlink_source_file_deploys_link_to_logical_source_path"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("real.txt"), b"copy me").unwrap();
        symlink("real.txt", source.join("link.txt")).unwrap();
        "[portal]\n\"link.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, target: &Path| {
        let file = target.join("target.txt");
        assert!(fs::symlink_metadata(&file).unwrap().is_file());
        assert_eq!(fs::read(&file).unwrap(), b"copy me");
        let record = record_of(&file).unwrap();
        assert_eq!(record.kind, Kind::File);
        assert_eq!(record.content_hash, Some(hash_bytes(b"copy me")));
    }
    ; "copy_rule_resolves_symlink_source_into_regular_file"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::create_dir_all(source.join("dir/sub")).unwrap();
        fs::write(source.join("dir/a.txt"), b"A").unwrap();
        fs::write(source.join("dir/sub/b.txt"), b"B").unwrap();
        "[portal]\n\"dir\" = \"dst\"\n"
    },
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("dst/a.txt")).unwrap(), b"A");
        assert_eq!(fs::read(target.join("dst/sub/b.txt")).unwrap(), b"B");
    }
    ; "literal_directory_portal_deploys_nested_files"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::create_dir_all(source.join("real/sub")).unwrap();
        fs::write(source.join("real/a.txt"), b"A").unwrap();
        fs::write(source.join("real/sub/b.txt"), b"B").unwrap();
        symlink("real", source.join("dir-link")).unwrap();
        "[portal]\n\"dir-link\" = \"dst\"\n"
    },
    |source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("dst/a.txt")).unwrap(), b"A");
        assert_eq!(fs::read(target.join("dst/sub/b.txt")).unwrap(), b"B");
        let link = target.join("dst/sub/b.txt");
        assert_eq!(
            fs::read_link(&link).unwrap(),
            source.join("dir-link/sub/b.txt")
        );
        let record = record_of(&link).unwrap();
        assert_eq!(record.kind, Kind::Symlink);
        assert_eq!(record.source_path, source.join("dir-link/sub/b.txt"));
    }
    ; "symlink_dir_portal_maps_children_under_logical_paths"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"copy me").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, target: &Path| {
        let file = target.join("target.txt");
        assert!(fs::symlink_metadata(&file).unwrap().is_file());
        assert_eq!(fs::read(&file).unwrap(), b"copy me");
        let record = record_of(&file).unwrap();
        assert_eq!(record.kind, Kind::File);
        assert_eq!(record.content_hash, Some(hash_bytes(b"copy me")));
    }
    ; "copy_rule_deploys_regular_file_with_recorded_hash"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("dotrift_data.toml"), "[variable]\nmessage = \"hi\"\n").unwrap();
        fs::write(source.join("greeting.txt"), "{{ message }}").unwrap();
        "[portal]\n\"greeting.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"template\" }\n"
    },
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"hi");
    }
    ; "template_rule_renders_variables_into_target"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("script.sh"), b"#!/bin/sh\n").unwrap();
        "[portal]\n\"script.sh\" = \"target.sh\"\n[rule]\n\"target.sh\" = { type = \"copy\", mode = \"600\" }\n"
    },
    |_source: &Path, target: &Path| {
        let mode = fs::metadata(target.join("target.sh")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    ; "mode_rule_sets_permissions_on_deployed_copy"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("dotrift_data.toml"), "[variable]\nname = \"hello\"\n").unwrap();
        fs::write(source.join("hello.txt"), b"payload").unwrap();
        "[portal]\n\"{{ name }}.txt\" = \"target.txt\"\n"
    },
    |_source: &Path, target: &Path| {
        let link = target.join("target.txt");
        assert!(link.is_symlink());
        assert_eq!(fs::read(&link).unwrap(), b"payload");
    }
    ; "templated_portal_key_resolves_from_data_file_variable"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::create_dir_all(source.join("empty")).unwrap();
        "[portal]\n\"empty\" = \"dst\"\n"
    },
    |_source: &Path, target: &Path| {
        assert!(fs::symlink_metadata(target.join("dst")).is_err());
        assert!(StateDatabase::open().unwrap().managed_paths().unwrap().is_empty());
    }
    ; "empty_directory_portal_deploys_nothing_successfully"
)]
fn first_apply_behaviors(
    setup: impl Fn(&Path, &Path) -> &'static str,
    assert: impl Fn(&Path, &Path),
) {
    let scenario = ApplyScenario::new(setup);
    let status = scenario.try_run().expect("apply failed");
    assert_eq!(status, ExitStatus::Success);
    assert(&scenario.source, &scenario.target);
}

#[test_case(
    |env: &TestEnv| {
        fs::write(env.source_dir().join("file.txt"), b"hello").unwrap();
        env.write_config("[portal]\n\"file.txt\" = \"target.txt\"\n");
    },
    ApplyOptions::default(),
    |env: &TestEnv| {
        let target = env.path("target");
        assert!(fs::symlink_metadata(&target).unwrap().is_dir());
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"hello");
    }
    ; "missing_target_root_is_created_and_populated"
)]
#[test_case(
    |env: &TestEnv| {
        fs::write(env.source_dir().join("file.txt"), b"hello").unwrap();
        env.write_config("[portal]\n\"file.txt\" = \"target.txt\"\n");
    },
    ApplyOptions {
        dry_run: true,
        ..Default::default()
    },
    |env: &TestEnv| {
        assert!(fs::symlink_metadata(env.path("target")).is_err());
        assert!(env.database().managed_paths().unwrap().is_empty());
    }
    ; "dry_run_fresh_deployment_changes_nothing"
)]
fn fresh_env_behaviors(setup: impl Fn(&TestEnv), options: ApplyOptions, assert: impl Fn(&TestEnv)) {
    let env = TestEnv::new();
    setup(&env);

    let status = dotrift::commands::apply::run_with_options(
        &env.source_dir(),
        Some(env.path("target")),
        options,
    )
    .unwrap();

    assert_eq!(status, ExitStatus::Success);
    assert(&env);
}

#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"old").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"new").unwrap();
        None
    },
    |source: &Path, target: &Path| {
        let link = target.join("target.txt");
        assert_eq!(fs::read_link(&link).unwrap(), source.join("file.txt"));
        assert_eq!(fs::read(&link).unwrap(), b"new");
        assert_eq!(record_of(&link).unwrap().source_path, source.join("file.txt"));
        assert_eq!(prompt_count(), 0);
    }
    ; "symlink_source_change_rewires_without_prompt"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"old").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"copy\" }\n"
    },
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"new").unwrap();
        None
    },
    |_source: &Path, target: &Path| {
        let file = target.join("target.txt");
        assert_eq!(fs::read(&file).unwrap(), b"new");
        assert_eq!(
            record_of(&file).unwrap().content_hash,
            Some(hash_bytes(b"new"))
        );
    }
    ; "copy_source_change_redeploys_with_updated_hash"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("dotrift_data.toml"), "[variable]\nmessage = \"hi\"\n").unwrap();
        fs::write(source.join("greeting.txt"), "{{ message }}").unwrap();
        "[portal]\n\"greeting.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"template\" }\n"
    },
    |source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"hi");
        fs::write(source.join("dotrift_data.toml"), "[variable]\nmessage = \"bye\"\n").unwrap();
        None
    },
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"bye");
    }
    ; "data_variable_change_rerenders_template"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(
            source.join("dotrift_data.toml"),
            "[variable]\nv = \"base\"\n[profile.work]\nv = \"over\"\n",
        )
        .unwrap();
        fs::write(source.join("out.txt"), "{{ v }}").unwrap();
        "[portal]\n\"out.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"template\" }\n"
    },
    |_source: &Path, _target: &Path| {
        StateDatabase::open().unwrap().activate_profile("work").unwrap();
        None
    },
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"over");
    }
    ; "profile_activation_between_runs_rerenders_override"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"content").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    |_source: &Path, _target: &Path| {
        Some("[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"copy\" }\n")
    },
    |_source: &Path, target: &Path| {
        let file = target.join("target.txt");
        assert!(fs::symlink_metadata(&file).unwrap().is_file());
        assert_eq!(fs::read(&file).unwrap(), b"content");
        let record = record_of(&file).unwrap();
        assert_eq!(record.kind, Kind::File);
        assert_eq!(record.content_hash, Some(hash_bytes(b"content")));
    }
    ; "adding_copy_rule_converts_symlink_to_plain_file"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"content").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, _target: &Path| {
        Some("[portal]\n\"file.txt\" = \"target.txt\"\n")
    },
    |source: &Path, target: &Path| {
        let link = target.join("target.txt");
        assert_eq!(fs::read_link(&link).unwrap(), source.join("file.txt"));
        let record = record_of(&link).unwrap();
        assert_eq!(record.kind, Kind::Symlink);
        assert_eq!(record.source_path, source.join("file.txt"));
    }
    ; "removing_copy_rule_reverts_target_to_symlink"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("script.sh"), b"#!/bin/sh\n").unwrap();
        "[portal]\n\"script.sh\" = \"target.sh\"\n[rule]\n\"target.sh\" = { type = \"copy\", mode = \"600\" }\n"
    },
    |_source: &Path, _target: &Path| {
        Some("[portal]\n\"script.sh\" = \"target.sh\"\n[rule]\n\"target.sh\" = { type = \"copy\", mode = \"644\" }\n")
    },
    |_source: &Path, target: &Path| {
        let mode = fs::metadata(target.join("target.sh")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o644);
    }
    ; "mode_change_updates_permissions_on_existing_copy"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("old.txt"), b"hello").unwrap();
        "[portal]\n\"old.txt\" = \"target.txt\"\n"
    },
    |source: &Path, _target: &Path| {
        fs::rename(source.join("old.txt"), source.join("new.txt")).unwrap();
        Some("[portal]\n\"new.txt\" = \"target.txt\"\n")
    },
    |source: &Path, target: &Path| {
        let link = target.join("target.txt");
        assert_eq!(fs::read_link(&link).unwrap(), source.join("new.txt"));
        assert_eq!(fs::read(&link).unwrap(), b"hello");
        assert_eq!(record_of(&link).unwrap().source_path, source.join("new.txt"));
    }
    ; "renamed_source_redirects_record_to_new_path"
)]
fn reapply_behaviors(
    setup: impl Fn(&Path, &Path) -> &'static str,
    modify: impl Fn(&Path, &Path) -> Option<&'static str>,
    assert: impl Fn(&Path, &Path),
) {
    let scenario = ApplyScenario::new(setup);
    scenario.run();
    scenario.rewrite(modify);
    scenario.run();
    assert(&scenario.source, &scenario.target);
}

#[test_case(
    Some(ObstructionChoice::Skip),
    |source: &Path, target: &Path| {
        fs::write(target.join("target.txt"), b"old").unwrap();
        fs::write(source.join("file.txt"), b"new").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    ExitStatus::Skipped,
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"old");
        assert_eq!(record_of(&target.join("target.txt")), None);
        assert_eq!(prompt_count(), 1);
    }
    ; "skipping_unmanaged_target_keeps_file_without_record_and_reports_skipped"
)]
#[test_case(
    Some(ObstructionChoice::Replace),
    |source: &Path, target: &Path| {
        fs::write(target.join("target.txt"), b"old").unwrap();
        fs::write(source.join("file.txt"), b"new").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    ExitStatus::Success,
    |source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"new");
        let record = record_of(&target.join("target.txt")).unwrap();
        assert_eq!(record.kind, Kind::Symlink);
        assert_eq!(record.source_path, source.join("file.txt"));
    }
    ; "replacing_unmanaged_target_deploys_and_records"
)]
#[test_case(
    Some(ObstructionChoice::ReplaceAll),
    |source: &Path, target: &Path| {
        fs::write(target.join("a.txt"), b"old-a").unwrap();
        fs::write(target.join("b.txt"), b"old-b").unwrap();
        fs::write(source.join("a.txt"), b"new-a").unwrap();
        fs::write(source.join("b.txt"), b"new-b").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"b.txt\"\n"
    },
    ExitStatus::Success,
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("a.txt")).unwrap(), b"new-a");
        assert_eq!(fs::read(target.join("b.txt")).unwrap(), b"new-b");
        assert_eq!(prompt_count(), 1);
    }
    ; "replace_all_latches_across_obstructions"
)]
#[test_case(
    None,
    |source: &Path, target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("z.txt"), b"Z").unwrap();
        fs::write(target.join("z.txt"), b"old").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"z.txt\" = \"z.txt\"\n"
    },
    ExitStatus::Cancelled,
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("a.txt")).unwrap(), b"A");
        assert!(record_of(&target.join("a.txt")).is_some());
        assert_eq!(fs::read(target.join("z.txt")).unwrap(), b"old");
        assert_eq!(record_of(&target.join("z.txt")), None);
        assert_eq!(prompt_count(), 1);
    }
    ; "cancelled_prompt_stops_run_preserving_completed_entries"
)]
fn unmanaged_target_obstruction_behaviors(
    choice: Option<ObstructionChoice>,
    setup: impl Fn(&Path, &Path) -> &'static str,
    expected_status: ExitStatus,
    assert: impl Fn(&Path, &Path),
) {
    let scenario = ApplyScenario::new(setup);
    if let Some(choice) = choice {
        set_prompt_choice(choice);
    }

    let status = scenario.try_run().expect("apply failed");

    assert_eq!(status, expected_status);
    assert(&scenario.source, &scenario.target);
}

#[test_case(
    ObstructionChoice::Skip,
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"original").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, target: &Path| {
        fs::write(target.join("target.txt"), b"tampered").unwrap();
    },
    ExitStatus::Skipped,
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"tampered");
        assert_eq!(
            record_of(&target.join("target.txt"))
                .unwrap()
                .content_hash,
            Some(hash_bytes(b"original"))
        );
        assert_eq!(prompt_count(), 1);
    }
    ; "skipping_tampered_managed_copy_retains_tamper_and_old_record"
)]
#[test_case(
    ObstructionChoice::Replace,
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"original").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, target: &Path| {
        fs::write(target.join("target.txt"), b"tampered").unwrap();
    },
    ExitStatus::Success,
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"original");
        assert_eq!(
            record_of(&target.join("target.txt"))
                .unwrap()
                .content_hash,
            Some(hash_bytes(b"original"))
        );
        assert_eq!(prompt_count(), 1);
    }
    ; "replacing_tampered_managed_copy_restores_content_and_hash"
)]
#[test_case(
    ObstructionChoice::Replace,
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"hello").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    |_source: &Path, target: &Path| {
        fs::remove_file(target.join("target.txt")).unwrap();
        fs::write(target.join("target.txt"), b"tampered").unwrap();
    },
    ExitStatus::Success,
    |source: &Path, target: &Path| {
        let link = target.join("target.txt");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_link(&link).unwrap(), source.join("file.txt"));
        assert_eq!(record_of(&link).unwrap().kind, Kind::Symlink);
        assert_eq!(prompt_count(), 1);
    }
    ; "replacing_tampered_managed_symlink_restores_link_and_kind"
)]
fn tampered_managed_target_behaviors(
    choice: ObstructionChoice,
    setup: impl Fn(&Path, &Path) -> &'static str,
    tamper: impl Fn(&Path, &Path),
    expected_status: ExitStatus,
    assert: impl Fn(&Path, &Path),
) {
    let scenario = ApplyScenario::new(setup);
    scenario.run();
    set_prompt_choice(choice);
    tamper(&scenario.source, &scenario.target);

    let status = scenario.try_run().expect("apply failed");

    assert_eq!(status, expected_status);
    assert(&scenario.source, &scenario.target);
}

#[test_case(
    Some(ObstructionChoice::Skip),
    |source: &Path, target: &Path| {
        fs::create_dir_all(target.join("a")).unwrap();
        fs::write(target.join("a/b"), b"block").unwrap();
        fs::write(source.join("file.txt"), b"new").unwrap();
        "[portal]\n\"file.txt\" = \"a/b/file.txt\"\n"
    },
    ExitStatus::Skipped,
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("a/b")).unwrap(), b"block");
        assert!(fs::symlink_metadata(target.join("a/b/file.txt")).is_err());
        assert_eq!(record_of(&target.join("a/b/file.txt")), None);
    }
    ; "skipping_parent_obstruction_leaves_blocker_intact"
)]
#[test_case(
    Some(ObstructionChoice::Replace),
    |source: &Path, target: &Path| {
        fs::create_dir_all(target.join("a")).unwrap();
        fs::write(target.join("a/b"), b"block").unwrap();
        fs::write(source.join("file.txt"), b"new").unwrap();
        "[portal]\n\"file.txt\" = \"a/b/file.txt\"\n"
    },
    ExitStatus::Success,
    |_source: &Path, target: &Path| {
        assert!(fs::metadata(target.join("a/b")).unwrap().is_dir());
        assert_eq!(fs::read(target.join("a/b/file.txt")).unwrap(), b"new");
        assert!(record_of(&target.join("a/b/file.txt")).is_some());
    }
    ; "replacing_parent_obstruction_deploys_nested_entry"
)]
#[test_case(
    None,
    |source: &Path, target: &Path| {
        fs::create_dir_all(target.join("real")).unwrap();
        symlink("real", target.join("link")).unwrap();
        fs::write(source.join("file.txt"), b"behind").unwrap();
        "[portal]\n\"file.txt\" = \"link/file.txt\"\n"
    },
    ExitStatus::Success,
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("real/file.txt")).unwrap(), b"behind");
        assert_eq!(fs::read(target.join("link/file.txt")).unwrap(), b"behind");
        assert!(record_of(&target.join("link/file.txt")).is_some());
        assert_eq!(prompt_count(), 0);
    }
    ; "directory_symlink_parent_is_traversed_for_deployment"
)]
#[test_case(
    Some(ObstructionChoice::Replace),
    |source: &Path, target: &Path| {
        fs::write(target.parent().unwrap().join("outside.txt"), b"safe").unwrap();
        symlink("../outside.txt", target.join("broken")).unwrap();
        fs::write(source.join("file.txt"), b"new").unwrap();
        "[portal]\n\"file.txt\" = \"broken/file.txt\"\n"
    },
    ExitStatus::Success,
    |_source: &Path, target: &Path| {
        let outside = target.parent().unwrap().join("outside.txt");
        assert_eq!(fs::read(outside).unwrap(), b"safe");
        assert!(fs::metadata(target.join("broken")).unwrap().is_dir());
        assert_eq!(fs::read(target.join("broken/file.txt")).unwrap(), b"new");
        assert!(record_of(&target.join("broken/file.txt")).is_some());
    }
    ; "dangling_parent_symlink_is_replaced_as_link_only"
)]
fn obstructed_parent_path_behaviors(
    choice: Option<ObstructionChoice>,
    setup: impl Fn(&Path, &Path) -> &'static str,
    expected_status: ExitStatus,
    assert: impl Fn(&Path, &Path),
) {
    let scenario = ApplyScenario::new(setup);
    if let Some(choice) = choice {
        set_prompt_choice(choice);
    }

    let status = scenario.try_run().expect("apply failed");

    assert_eq!(status, expected_status);
    assert(&scenario.source, &scenario.target);
}

#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n"
    },
    |_source: &Path, _target: &Path| Some("[portal]\n"),
    false,
    |_source: &Path, target: &Path| {
        assert!(fs::symlink_metadata(target.join("a.txt")).is_err());
        assert_eq!(record_of(&target.join("a.txt")), None);
    }
    ; "clean_up_removes_stale_symlink"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n[rule]\n\"a.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, _target: &Path| Some("[portal]\n"),
    false,
    |_source: &Path, target: &Path| {
        assert!(fs::symlink_metadata(target.join("a.txt")).is_err());
        assert_eq!(record_of(&target.join("a.txt")), None);
    }
    ; "clean_up_removes_stale_copy"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("dotrift_data.toml"), "[variable]\nname = \"x\"\n").unwrap();
        fs::write(source.join("a.txt"), "{{ name }}").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n[rule]\n\"a.txt\" = { type = \"template\" }\n"
    },
    |_source: &Path, _target: &Path| Some("[portal]\n"),
    false,
    |_source: &Path, target: &Path| {
        assert!(fs::symlink_metadata(target.join("a.txt")).is_err());
        assert_eq!(record_of(&target.join("a.txt")), None);
    }
    ; "clean_up_removes_stale_template"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::create_dir_all(source.join("dir/sub")).unwrap();
        fs::write(source.join("dir/a.txt"), b"A").unwrap();
        fs::write(source.join("dir/sub/b.txt"), b"B").unwrap();
        "[portal]\n\"dir\" = \"dst\"\n"
    },
    |_source: &Path, _target: &Path| Some("[portal]\n"),
    false,
    |_source: &Path, target: &Path| {
        assert!(fs::symlink_metadata(target.join("dst/a.txt")).is_err());
        assert!(fs::symlink_metadata(target.join("dst/sub/b.txt")).is_err());
        assert_eq!(record_of(&target.join("dst/a.txt")), None);
        assert_eq!(record_of(&target.join("dst/sub/b.txt")), None);
    }
    ; "clean_up_removes_every_file_under_removed_directory_portal"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("keep.txt"), b"K").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"keep.txt\" = \"keep.txt\"\n"
    },
    |_source: &Path, _target: &Path| Some("[portal]\n\"keep.txt\" = \"keep.txt\"\n"),
    false,
    |_source: &Path, target: &Path| {
        assert!(fs::symlink_metadata(target.join("a.txt")).is_err());
        assert_eq!(fs::read(target.join("keep.txt")).unwrap(), b"K");
        assert!(record_of(&target.join("keep.txt")).is_some());
    }
    ; "clean_up_keeps_desired_target_and_removes_stale"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n"
    },
    |_source: &Path, _target: &Path| None,
    false,
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("a.txt")).unwrap(), b"A");
        assert!(record_of(&target.join("a.txt")).is_some());
    }
    ; "clean_up_is_no_op_when_nothing_is_stale"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"original").unwrap();
        "[portal]\n\"file.txt\" = \"file.txt\"\n[rule]\n\"file.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, target: &Path| {
        fs::write(target.join("file.txt"), b"tampered").unwrap();
        Some("[portal]\n")
    },
    false,
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("file.txt")).unwrap(), b"tampered");
        assert_eq!(record_of(&target.join("file.txt")), None);
    }
    ; "clean_up_relinquishes_tampered_stale_path"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"A").unwrap();
        "[portal]\n\"file.txt\" = \"file.txt\"\n[rule]\n\"file.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, target: &Path| {
        fs::remove_file(target.join("file.txt")).unwrap();
        Some("[portal]\n")
    },
    false,
    |_source: &Path, target: &Path| {
        assert!(fs::symlink_metadata(target.join("file.txt")).is_err());
        assert_eq!(record_of(&target.join("file.txt")), None);
    }
    ; "clean_up_relinquishes_missing_target"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::create_dir(source.join("dir")).unwrap();
        fs::write(source.join("dir/a.txt"), b"A").unwrap();
        "[portal]\n\"dir\" = \"dst\"\n"
    },
    |_source: &Path, target: &Path| {
        fs::remove_dir_all(target.join("dst")).unwrap();
        fs::write(target.join("dst"), b"replacement").unwrap();
        Some("[portal]\n")
    },
    false,
    |_source: &Path, target: &Path| {
        assert!(fs::symlink_metadata(target.join("dst")).unwrap().is_file());
        assert_eq!(fs::read(target.join("dst")).unwrap(), b"replacement");
        assert_eq!(record_of(&target.join("dst/a.txt")), None);
    }
    ; "deployed_dir_replaced_by_file_relinquishes_child_records_untouched"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"A").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    |_source: &Path, target: &Path| {
        fs::remove_file(target.join("target.txt")).unwrap();
        fs::create_dir(target.join("target.txt")).unwrap();
        Some("[portal]\n")
    },
    false,
    |_source: &Path, target: &Path| {
        assert!(fs::metadata(target.join("target.txt")).unwrap().is_dir());
        assert_eq!(record_of(&target.join("target.txt")), None);
    }
    ; "deployed_file_replaced_by_directory_is_relinquished_not_deleted"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"A").unwrap();
        "[portal]\n\"file.txt\" = \"a/b/file.txt\"\n[rule]\n\"a/b/file.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, _target: &Path| Some("[portal]\n"),
    true,
    |_source: &Path, target: &Path| {
        assert!(fs::symlink_metadata(target.join("a")).is_err());
        assert!(fs::symlink_metadata(target.join("a/b")).is_err());
        assert_eq!(record_of(&target.join("a/b/file.txt")), None);
    }
    ; "prune_empty_dirs_removes_emptied_parent_chain_up_to_root"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"A").unwrap();
        "[portal]\n\"file.txt\" = \"a/b/file.txt\"\n[rule]\n\"a/b/file.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, _target: &Path| Some("[portal]\n"),
    false,
    |_source: &Path, target: &Path| {
        assert!(fs::metadata(target.join("a/b")).unwrap().is_dir());
        assert!(fs::metadata(target.join("a")).unwrap().is_dir());
        assert!(fs::symlink_metadata(target.join("a/b/file.txt")).is_err());
    }
    ; "without_prune_flag_emptied_parents_remain"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"A").unwrap();
        fs::write(source.join("keep.txt"), b"K").unwrap();
        "[portal]\n\"file.txt\" = \"a/b/file.txt\"\n\"keep.txt\" = \"a/keep.txt\"\n[rule]\n\"a/b/file.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, _target: &Path| Some("[portal]\n\"keep.txt\" = \"a/keep.txt\"\n"),
    true,
    |_source: &Path, target: &Path| {
        assert!(fs::symlink_metadata(target.join("a/b")).is_err());
        assert!(fs::metadata(target.join("a")).unwrap().is_dir());
        assert_eq!(fs::read(target.join("a/keep.txt")).unwrap(), b"K");
        assert!(record_of(&target.join("a/keep.txt")).is_some());
    }
    ; "prune_stops_at_parent_holding_desired_target"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"A").unwrap();
        "[portal]\n\"file.txt\" = \"a/b/stale.txt\"\n[rule]\n\"a/b/stale.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, target: &Path| {
        fs::create_dir(target.join("a/b/keep")).unwrap();
        Some("[portal]\n")
    },
    true,
    |_source: &Path, target: &Path| {
        assert!(fs::symlink_metadata(target.join("a/b/stale.txt")).is_err());
        assert_eq!(record_of(&target.join("a/b/stale.txt")), None);
        assert!(fs::metadata(target.join("a/b/keep")).unwrap().is_dir());
        assert!(fs::metadata(target.join("a/b")).unwrap().is_dir());
        assert!(fs::metadata(target.join("a")).unwrap().is_dir());
    }
    ; "prune_stops_at_directory_holding_unmanaged_subdirectory"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"A").unwrap();
        "[portal]\n\"file.txt\" = \"sub/stale.txt\"\n[rule]\n\"sub/stale.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, target: &Path| {
        fs::remove_file(target.join("sub/stale.txt")).unwrap();
        fs::remove_dir(target.join("sub")).unwrap();
        fs::create_dir(target.join("real")).unwrap();
        fs::write(target.join("real/stale.txt"), b"A").unwrap();
        symlink("real", target.join("sub")).unwrap();
        Some("[portal]\n")
    },
    true,
    |_source: &Path, target: &Path| {
        assert!(fs::symlink_metadata(target.join("real/stale.txt")).is_err());
        assert_eq!(record_of(&target.join("sub/stale.txt")), None);
        assert!(
            fs::symlink_metadata(target.join("sub"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(fs::metadata(target.join("real")).unwrap().is_dir());
    }
    ; "prune_stops_at_symlink_parent_leaving_link_and_target_intact"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"A").unwrap();
        "[portal]\n\"file.txt\" = \"sub/file.txt\"\n[rule]\n\"sub/file.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, target: &Path| {
        fs::remove_file(target.join("sub/file.txt")).unwrap();
        fs::remove_dir(target.join("sub")).unwrap();
        fs::create_dir(target.join("real")).unwrap();
        fs::write(target.join("real/file.txt"), b"A").unwrap();
        symlink("real", target.join("sub")).unwrap();
        Some("[portal]\n")
    },
    false,
    |_source: &Path, target: &Path| {
        assert_eq!(record_of(&target.join("sub/file.txt")), None);
        assert!(
            fs::symlink_metadata(target.join("sub"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(fs::symlink_metadata(target.join("real/file.txt")).is_err());
    }
    ; "clean_up_unlinks_stale_managed_file_behind_symlink_parent"
)]
fn clean_up_behaviors(
    setup: impl Fn(&Path, &Path) -> &'static str,
    modify: impl Fn(&Path, &Path) -> Option<&'static str>,
    prune: bool,
    assert: impl Fn(&Path, &Path),
) {
    let scenario = ApplyScenario::new(setup);
    scenario.run();
    scenario.rewrite(modify);
    scenario.run_with_options(ApplyOptions {
        clean_up: true,
        prune_empty_dirs: prune,
        ..Default::default()
    });
    assert(&scenario.source, &scenario.target);
}

#[test]
fn skipped_entry_blocks_clean_up_for_that_run() {
    let scenario = ApplyScenario::new(|source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"b.txt\"\n[rule]\n\"a.txt\" = { type = \"copy\" }\n"
    });
    scenario.run();
    fs::write(scenario.target.join("a.txt"), b"tampered").unwrap();
    scenario
        .write_config("[portal]\n\"a.txt\" = \"a.txt\"\n[rule]\n\"a.txt\" = { type = \"copy\" }\n");

    set_prompt_choice(ObstructionChoice::Skip);
    let status = scenario
        .try_run_with_options(ApplyOptions {
            clean_up: true,
            ..Default::default()
        })
        .expect("apply failed");

    assert_eq!(status, ExitStatus::Skipped);
    assert!(scenario.target.join("b.txt").is_symlink());
    assert!(record_of(&scenario.target.join("b.txt")).is_some());

    set_prompt_choice(ObstructionChoice::Replace);
    let status = scenario
        .try_run_with_options(ApplyOptions {
            clean_up: true,
            ..Default::default()
        })
        .expect("apply failed");

    assert_eq!(status, ExitStatus::Success);
    assert_eq!(fs::read(scenario.target.join("a.txt")).unwrap(), b"A");
    assert!(fs::symlink_metadata(scenario.target.join("b.txt")).is_err());
    assert_eq!(record_of(&scenario.target.join("b.txt")), None);
}

#[test]
fn records_outside_current_target_root_are_never_candidates() {
    let scenario = ApplyScenario::new(|source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n"
    });
    let foreign_dir = scenario.env.path("other");
    fs::create_dir_all(&foreign_dir).unwrap();
    let foreign = foreign_dir.join("stale.txt");
    StateDatabase::open()
        .unwrap()
        .put(&dotrift::record!(
            s,
            foreign.clone(),
            scenario.source.join("a.txt")
        ))
        .unwrap();

    scenario.run_with_options(ApplyOptions {
        clean_up: true,
        ..Default::default()
    });

    assert!(record_of(&foreign).is_some());
}

#[test_case(
    |source: &Path, target: &Path| {
        fs::write(target.join("target.txt"), b"unmanaged").unwrap();
        fs::write(source.join("file.txt"), b"new").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    |_scenario: &ApplyScenario| {},
    ApplyOptions {
        dry_run: true,
        ..Default::default()
    },
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"unmanaged");
        assert_eq!(record_of(&target.join("target.txt")), None);
        assert_eq!(prompt_count(), 0);
    }
    ; "dry_run_leaves_obstruction_unprompted_and_untouched"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"original").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"copy\" }\n"
    },
    |scenario: &ApplyScenario| {
        scenario.run();
        fs::write(scenario.target.join("target.txt"), b"tampered").unwrap();
    },
    ApplyOptions {
        dry_run: true,
        ..Default::default()
    },
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"tampered");
        assert_eq!(
            record_of(&target.join("target.txt"))
                .unwrap()
                .content_hash,
            Some(hash_bytes(b"original"))
        );
        assert_eq!(prompt_count(), 0);
    }
    ; "dry_run_leaves_tampered_managed_target_untouched"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"b.txt\"\n"
    },
    |scenario: &ApplyScenario| {
        scenario.run();
        scenario.write_config("[portal]\n\"a.txt\" = \"a.txt\"\n");
    },
    ApplyOptions {
        dry_run: true,
        clean_up: true,
        ..Default::default()
    },
    |_source: &Path, target: &Path| {
        assert!(target.join("b.txt").is_symlink());
        assert!(record_of(&target.join("b.txt")).is_some());
    }
    ; "dry_run_with_clean_up_keeps_stale_target_and_record"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"F").unwrap();
        "[portal]\n\"file.txt\" = \"a/b/stale.txt\"\n"
    },
    |scenario: &ApplyScenario| scenario.run(),
    ApplyOptions {
        dry_run: true,
        clean_up: true,
        prune_empty_dirs: true,
        ..Default::default()
    },
    |_source: &Path, target: &Path| {
        assert!(fs::metadata(target.join("a/b")).unwrap().is_dir());
        assert!(target.join("a/b/stale.txt").is_symlink());
        assert!(record_of(&target.join("a/b/stale.txt")).is_some());
    }
    ; "dry_run_with_clean_up_and_prune_keeps_stale_directories"
)]
fn dry_run_preservation_behaviors(
    setup: impl Fn(&Path, &Path) -> &'static str,
    prepare: impl Fn(&ApplyScenario),
    options: ApplyOptions,
    assert: impl Fn(&Path, &Path),
) {
    let scenario = ApplyScenario::new(setup);
    prepare(&scenario);

    let status = scenario
        .try_run_with_options(options)
        .expect("apply failed");

    assert_eq!(status, ExitStatus::Success);
    assert(&scenario.source, &scenario.target);
}

#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("good.txt"), b"G").unwrap();
        "[portal]\n\"good.txt\" = \"good-target.txt\"\n\"missing.txt\" = \"x.txt\"\n"
    },
    |_scenario: &ApplyScenario| {},
    "literal portal source",
    |_source: &Path, target: &Path| {
        assert!(fs::symlink_metadata(target.join("good-target.txt")).is_err());
        assert!(
            StateDatabase::open()
                .unwrap()
                .managed_paths()
                .unwrap()
                .is_empty()
        );
    }
    ; "missing_literal_portal_source_fails_before_any_change"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("good.txt"), b"G").unwrap();
        fs::write(source.join("bad.tmpl"), "{{ missing }}").unwrap();
        "[portal]\n\"good.txt\" = \"a.txt\"\n\"bad.tmpl\" = \"b.txt\"\n[rule]\n\"a.txt\" = { type = \"copy\" }\n\"b.txt\" = { type = \"template\" }\n"
    },
    |_scenario: &ApplyScenario| {},
    "undefined variable",
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("a.txt")).unwrap(), b"G");
        assert!(record_of(&target.join("a.txt")).is_some());
        assert!(fs::symlink_metadata(target.join("b.txt")).is_err());
        assert_eq!(record_of(&target.join("b.txt")), None);
    }
    ; "template_render_failure_mid_run_preserves_completed_entries"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"hello").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    |scenario: &ApplyScenario| {
        scenario.run();
        fs::remove_file(scenario.source.join("file.txt")).unwrap();
    },
    "literal portal source",
    |_source: &Path, target: &Path| {
        assert!(target.join("target.txt").is_symlink());
    }
    ; "deleting_source_between_runs_fails_preflight"
)]
fn failing_apply_behaviors(
    setup: impl Fn(&Path, &Path) -> &'static str,
    prepare: impl Fn(&ApplyScenario),
    needle: &'static str,
    assert: impl Fn(&Path, &Path),
) {
    let scenario = ApplyScenario::new(setup);
    prepare(&scenario);

    let error = scenario.try_run().unwrap_err();

    assert_error_chain(&error, needle);
    assert(&scenario.source, &scenario.target);
}
