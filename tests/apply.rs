mod common;

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use dotrift::commands::apply::{ApplyOptions, ObstructionChoice, set_prompt_choice};
use dotrift::hash::hash_bytes;
use dotrift::state::{Kind, StateDatabase};
use test_case::test_case;

use common::{ApplyScenario, TestEnv, prompt_count, snapshot_settings, test_name};

#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"hello").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    |_source: &Path, target: &Path| {
        assert!(target.join("target.txt").is_symlink());
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"hello");
    }
    ; "single_file_portal_deploys_symlink"
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
        fs::create_dir_all(source.join("configs/sub")).unwrap();
        fs::write(source.join("configs/a.txt"), b"A").unwrap();
        fs::write(source.join("configs/sub/b.txt"), b"B").unwrap();
        "[portal]\n\"configs/**\" = \"cfg\"\n"
    },
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("cfg/a.txt")).unwrap(), b"A");
        assert_eq!(fs::read(target.join("cfg/sub/b.txt")).unwrap(), b"B");
    }
    ; "glob_portal_deploys_files_with_prefix_stripped"
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
        let record = StateDatabase::open().unwrap().record(&file).unwrap().unwrap();
        assert_eq!(record.kind, Kind::File);
        assert_eq!(record.content_hash, Some(hash_bytes(b"copy me")));
    }
    ; "copy_rule_deploys_plain_file_with_recorded_hash"
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
        let mode = fs::metadata(target.join("target.sh"))
            .unwrap()
            .permissions()
            .mode();
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
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"payload");
    }
    ; "templated_source_path_resolves_from_data_file_variable"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        StateDatabase::open()
            .unwrap()
            .activate_profile("work")
            .unwrap();
        fs::write(
            source.join("dotrift_data.toml"),
            "[variable]\nv = \"base\"\n[profile.work]\nv = \"over\"\n",
        )
        .unwrap();
        fs::write(source.join("out.txt"), "{{ v }}").unwrap();
        "[portal]\n\"out.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"template\" }\n"
    },
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"over");
    }
    ; "active_profile_value_overrides_base_value"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        StateDatabase::open()
            .unwrap()
            .activate_profile("gone")
            .unwrap();
        fs::write(source.join("dotrift_data.toml"), "[variable]\nv = \"base\"\n").unwrap();
        fs::write(source.join("out.txt"), "{{ v }}").unwrap();
        "[portal]\n\"out.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"template\" }\n"
    },
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"base");
    }
    ; "active_profile_without_definition_falls_back_to_base"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join(".dotriftignore"), "target.txt\n").unwrap();
        fs::write(source.join("file.txt"), b"hello").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    |_source: &Path, target: &Path| {
        assert!(fs::symlink_metadata(target.join("target.txt")).is_err());
    }
    ; "dotriftignore_excludes_deploying_matching_target"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join(".dotriftignore"), "*.txt\n!keep.txt\n").unwrap();
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("keep.txt"), b"K").unwrap();
        "[portal]\n\"*\" = \"out\"\n"
    },
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("out/keep.txt")).unwrap(), b"K");
        assert!(fs::symlink_metadata(target.join("out/a.txt")).is_err());
    }
    ; "dotriftignore_negation_reincludes_matching_file"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("dotrift_data.toml"), "[variable]\n").unwrap();
        fs::write(source.join(".dotriftignore"), "\n").unwrap();
        "[portal]\n\"**\" = \".\"\n"
    },
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("a.txt")).unwrap(), b"A");
        assert!(fs::symlink_metadata(target.join("dotrift.toml")).is_err());
        assert!(fs::symlink_metadata(target.join("dotrift_data.toml")).is_err());
        assert!(fs::symlink_metadata(target.join(".dotriftignore")).is_err());
    }
    ; "control_files_are_implicitly_excluded_from_deploy"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join(".dotriftignore"), "one.txt\n").unwrap();
        fs::write(source.join("file.txt"), b"data").unwrap();
        "[portal]\n\"file.txt\" = \"one.txt\"\n\"*.txt\" = \"two.txt\"\n"
    },
    |_source: &Path, target: &Path| {
        assert_eq!(
            fs::read(target.join("two.txt/file.txt")).unwrap(),
            b"data"
        );
        assert!(fs::symlink_metadata(target.join("one.txt")).is_err());
    }
    ; "dotriftignore_applies_to_target_path_not_source"
)]
#[test_case(
    |source: &Path, target: &Path| {
        set_prompt_choice(ObstructionChoice::Skip);
        fs::write(target.join("target.txt"), b"old").unwrap();
        fs::write(source.join("file.txt"), b"new").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"old");
        assert_eq!(
            StateDatabase::open()
                .unwrap()
                .record(&target.join("target.txt"))
                .unwrap(),
            None
        );
    }
    ; "skipping_unmanaged_target_leaves_original_and_record"
)]
#[test_case(
    |source: &Path, target: &Path| {
        set_prompt_choice(ObstructionChoice::Replace);
        fs::write(target.join("target.txt"), b"old").unwrap();
        fs::write(source.join("file.txt"), b"new").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"new");
        assert!(
            StateDatabase::open()
                .unwrap()
                .record(&target.join("target.txt"))
                .unwrap()
                .is_some()
        );
    }
    ; "replacing_unmanaged_target_deploys_and_records_file"
)]
#[test_case(
    |source: &Path, target: &Path| {
        set_prompt_choice(ObstructionChoice::Skip);
        fs::create_dir_all(target.join("a")).unwrap();
        fs::write(target.join("a/b"), b"block").unwrap();
        fs::write(source.join("file.txt"), b"new").unwrap();
        "[portal]\n\"file.txt\" = \"a/b/file.txt\"\n"
    },
    |_source: &Path, target: &Path| {
        assert!(fs::metadata(target.join("a/b")).unwrap().is_file());
        assert_eq!(fs::read(target.join("a/b")).unwrap(), b"block");
        assert!(fs::symlink_metadata(target.join("a/b/file.txt")).is_err());
    }
    ; "skipping_parent_obstruction_keeps_blocking_file"
)]
#[test_case(
    |source: &Path, target: &Path| {
        set_prompt_choice(ObstructionChoice::Replace);
        fs::create_dir_all(target.join("a")).unwrap();
        fs::write(target.join("a/b"), b"block").unwrap();
        fs::write(source.join("file.txt"), b"new").unwrap();
        "[portal]\n\"file.txt\" = \"a/b/file.txt\"\n"
    },
    |_source: &Path, target: &Path| {
        assert!(fs::metadata(target.join("a/b")).unwrap().is_dir());
        assert_eq!(fs::read(target.join("a/b/file.txt")).unwrap(), b"new");
        assert!(
            StateDatabase::open()
                .unwrap()
                .record(&target.join("a/b/file.txt"))
                .unwrap()
                .is_some()
        );
    }
    ; "replacing_parent_obstruction_deploys_nested_file"
)]
#[test_case(
    |source: &Path, target: &Path| {
        set_prompt_choice(ObstructionChoice::ReplaceAll);
        fs::write(target.join("a.txt"), b"old-a").unwrap();
        fs::write(target.join("b.txt"), b"old-b").unwrap();
        fs::write(source.join("a.txt"), b"new-a").unwrap();
        fs::write(source.join("b.txt"), b"new-b").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"b.txt\"\n"
    },
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("a.txt")).unwrap(), b"new-a");
        assert_eq!(fs::read(target.join("b.txt")).unwrap(), b"new-b");
        assert_eq!(prompt_count(), 1);
    }
    ; "replace_all_choice_latches_for_subsequent_obstructions"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::create_dir_all(source.join("empty")).unwrap();
        "[portal]\n\"empty\" = \"dst\"\n"
    },
    |_source: &Path, _target: &Path| {}
    ; "deploying_empty_directory_succeeds"
)]
fn first_apply_behaviors(
    setup: impl Fn(&Path, &Path) -> &'static str,
    assert: impl Fn(&Path, &Path),
) {
    let scenario = ApplyScenario::new(setup);
    scenario.run();
    assert(&scenario.source, &scenario.target);
}

#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"hello").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    |_source: &Path, _target: &Path| None,
    |source: &Path, target: &Path| {
        let file = target.join("target.txt");
        assert!(file.is_symlink());
        assert_eq!(fs::read(&file).unwrap(), b"hello");
        let record = StateDatabase::open()
            .unwrap()
            .record(&file)
            .unwrap()
            .unwrap();
        assert_eq!(record.kind, Kind::Symlink);
        assert_eq!(record.source_path, source.join("file.txt"));
    }
    ; "reapplying_unchanged_config_is_idempotent"
)]
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
        let file = target.join("target.txt");
        assert!(file.is_symlink());
        assert_eq!(fs::read_link(&file).unwrap(), source.join("file.txt"));
        assert_eq!(fs::read(&file).unwrap(), b"new");
        let record = StateDatabase::open()
            .unwrap()
            .record(&file)
            .unwrap()
            .unwrap();
        assert_eq!(record.kind, Kind::Symlink);
        assert_eq!(record.source_path, source.join("file.txt"));
        assert_eq!(prompt_count(), 0);
    }
    ; "source_content_change_rewires_symlink_without_prompt"
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
        let record = StateDatabase::open()
            .unwrap()
            .record(&file)
            .unwrap()
            .unwrap();
        assert_eq!(record.kind, Kind::File);
        assert_eq!(record.content_hash, Some(hash_bytes(b"new")));
    }
    ; "source_change_redeploys_copy_with_updated_hash"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(
            source.join("dotrift_data.toml"),
            "[variable]\nmessage = \"hi\"\n",
        )
        .unwrap();
        fs::write(source.join("greeting.txt"), "{{ message }}").unwrap();
        "[portal]\n\"greeting.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"template\" }\n"
    },
    |source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"hi");
        fs::write(
            source.join("dotrift_data.toml"),
            "[variable]\nmessage = \"bye\"\n",
        )
        .unwrap();
        None
    },
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"bye");
    }
    ; "variable_change_rerenders_template_output"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"content").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    |_source: &Path, target: &Path| {
        assert!(target.join("target.txt").is_symlink());
        Some("[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"copy\" }\n")
    },
    |_source: &Path, target: &Path| {
        let file = target.join("target.txt");
        assert!(fs::symlink_metadata(&file).unwrap().is_file());
        assert_eq!(fs::read(&file).unwrap(), b"content");
        let record = StateDatabase::open()
            .unwrap()
            .record(&file)
            .unwrap()
            .unwrap();
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
    |_source: &Path, target: &Path| {
        assert!(fs::symlink_metadata(target.join("target.txt")).unwrap().is_file());
        Some("[portal]\n\"file.txt\" = \"target.txt\"\n")
    },
    |source: &Path, target: &Path| {
        let file = target.join("target.txt");
        assert!(file.is_symlink());
        assert_eq!(fs::read_link(&file).unwrap(), source.join("file.txt"));
        let record = StateDatabase::open()
            .unwrap()
            .record(&file)
            .unwrap()
            .unwrap();
        assert_eq!(record.kind, Kind::Symlink);
        assert_eq!(record.source_path, source.join("file.txt"));
    }
    ; "removing_copy_rule_reverts_target_to_symlink"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(
            source.join("dotrift_data.toml"),
            "[variable]\nname = \"x\"\n",
        )
        .unwrap();
        fs::write(source.join("file.txt"), "{{ name }}").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"{{ name }}");
        Some("[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"template\" }\n")
    },
    |_source: &Path, target: &Path| {
        let file = target.join("target.txt");
        assert_eq!(fs::read(&file).unwrap(), b"x");
        let record = StateDatabase::open()
            .unwrap()
            .record(&file)
            .unwrap()
            .unwrap();
        assert_eq!(record.kind, Kind::File);
        assert_eq!(record.content_hash, Some(hash_bytes(b"x")));
    }
    ; "switching_copy_rule_to_template_renders_output"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("script.sh"), b"#!/bin/sh\n").unwrap();
        "[portal]\n\"script.sh\" = \"target.sh\"\n[rule]\n\"target.sh\" = { type = \"copy\", mode = \"600\" }\n"
    },
    |_source: &Path, target: &Path| {
        let mode = fs::metadata(target.join("target.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        Some("[portal]\n\"script.sh\" = \"target.sh\"\n[rule]\n\"target.sh\" = { type = \"copy\", mode = \"644\" }\n")
    },
    |_source: &Path, target: &Path| {
        let mode = fs::metadata(target.join("target.sh"))
            .unwrap()
            .permissions()
            .mode();
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
        let file = target.join("target.txt");
        assert!(file.is_symlink());
        assert_eq!(fs::read_link(&file).unwrap(), source.join("new.txt"));
        assert_eq!(fs::read(&file).unwrap(), b"hello");
        let record = StateDatabase::open()
            .unwrap()
            .record(&file)
            .unwrap()
            .unwrap();
        assert_eq!(record.source_path, source.join("new.txt"));
    }
    ; "renamed_source_redirects_symlink_to_new_path"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"hello").unwrap();
        "[portal]\n\"file.txt\" = \"old.txt\"\n"
    },
    |_source: &Path, _target: &Path| Some("[portal]\n\"file.txt\" = \"new.txt\"\n"),
    |source: &Path, target: &Path| {
        let new = target.join("new.txt");
        assert!(new.is_symlink());
        assert_eq!(fs::read(&new).unwrap(), b"hello");
        let old = target.join("old.txt");
        assert!(old.is_symlink());
        assert_eq!(fs::read_link(&old).unwrap(), source.join("file.txt"));
        assert!(
            StateDatabase::open()
                .unwrap()
                .record(&old)
                .unwrap()
                .is_some()
        );
    }
    ; "moved_target_path_leaves_old_managed_path_stale"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"b.txt\"\n"
    },
    |_source: &Path, _target: &Path| Some("[portal]\n\"a.txt\" = \"a.txt\"\n"),
    |source: &Path, target: &Path| {
        let file = target.join("b.txt");
        assert!(file.is_symlink());
        assert_eq!(fs::read_link(&file).unwrap(), source.join("b.txt"));
        assert!(StateDatabase::open().unwrap().record(&file).unwrap().is_some());
    }
    ; "removing_portal_entry_leaves_target_path_stale"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n"
    },
    |source: &Path, _target: &Path| {
        fs::write(source.join("b.txt"), b"B").unwrap();
        Some("[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"b.txt\"\n")
    },
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("a.txt")).unwrap(), b"A");
        assert_eq!(fs::read(target.join("b.txt")).unwrap(), b"B");
    }
    ; "adding_portal_entry_deploys_alongside_existing"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        "[portal]\n\"*.txt\" = \".\"\n"
    },
    |source: &Path, _target: &Path| {
        fs::write(source.join("b.md"), b"B").unwrap();
        Some("[portal]\n\"**\" = \".\"\n")
    },
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("a.txt")).unwrap(), b"A");
        assert_eq!(fs::read(target.join("b.md")).unwrap(), b"B");
        assert!(fs::symlink_metadata(target.join("dotrift.toml")).is_err());
    }
    ; "widened_glob_deploys_newly_matched_file"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"hello").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    |source: &Path, _target: &Path| {
        fs::write(source.join(".dotriftignore"), "target.txt\n").unwrap();
        None
    },
    |source: &Path, target: &Path| {
        let file = target.join("target.txt");
        assert!(file.is_symlink());
        assert_eq!(fs::read_link(&file).unwrap(), source.join("file.txt"));
        assert!(StateDatabase::open().unwrap().record(&file).unwrap().is_some());
    }
    ; "added_ignore_rule_makes_existing_target_stale"
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
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"base");
        StateDatabase::open()
            .unwrap()
            .activate_profile("work")
            .unwrap();
        None
    },
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"over");
    }
    ; "activating_profile_between_runs_rerenders_override"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"original").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, target: &Path| {
        set_prompt_choice(ObstructionChoice::Replace);
        fs::write(target.join("target.txt"), b"tampered").unwrap();
        None
    },
    |_source: &Path, target: &Path| {
        let file = target.join("target.txt");
        assert_eq!(fs::read(&file).unwrap(), b"original");
        let record = StateDatabase::open()
            .unwrap()
            .record(&file)
            .unwrap()
            .unwrap();
        assert_eq!(record.content_hash, Some(hash_bytes(b"original")));
        assert_eq!(prompt_count(), 1);
    }
    ; "replacing_tampered_copy_restores_source_content"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"original").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, target: &Path| {
        set_prompt_choice(ObstructionChoice::Skip);
        fs::write(target.join("target.txt"), b"tampered").unwrap();
        None
    },
    |_source: &Path, target: &Path| {
        let file = target.join("target.txt");
        assert_eq!(fs::read(&file).unwrap(), b"tampered");
        let record = StateDatabase::open()
            .unwrap()
            .record(&file)
            .unwrap()
            .unwrap();
        assert_eq!(record.content_hash, Some(hash_bytes(b"original")));
        assert_eq!(prompt_count(), 1);
    }
    ; "skipping_tampered_copy_keeps_record_and_tamper"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"hello").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    |_source: &Path, target: &Path| {
        set_prompt_choice(ObstructionChoice::Replace);
        fs::remove_file(target.join("target.txt")).unwrap();
        fs::write(target.join("target.txt"), b"tampered").unwrap();
        None
    },
    |source: &Path, target: &Path| {
        let file = target.join("target.txt");
        assert!(file.is_symlink());
        assert_eq!(fs::read_link(&file).unwrap(), source.join("file.txt"));
        assert_eq!(fs::read(&file).unwrap(), b"hello");
        let record = StateDatabase::open()
            .unwrap()
            .record(&file)
            .unwrap()
            .unwrap();
        assert_eq!(record.kind, Kind::Symlink);
        assert_eq!(record.source_path, source.join("file.txt"));
        assert_eq!(prompt_count(), 1);
    }
    ; "replacing_tampered_symlink_restores_link"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"hello").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, target: &Path| {
        fs::remove_file(target.join("target.txt")).unwrap();
        None
    },
    |_source: &Path, target: &Path| {
        let file = target.join("target.txt");
        assert_eq!(fs::read(&file).unwrap(), b"hello");
        let record = StateDatabase::open()
            .unwrap()
            .record(&file)
            .unwrap()
            .unwrap();
        assert_eq!(record.kind, Kind::File);
        assert_eq!(record.content_hash, Some(hash_bytes(b"hello")));
    }
    ; "deleted_copy_target_redeploys_despite_stale_record"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"hello").unwrap();
        "[portal]\n\"file.txt\" = \"sub/file.txt\"\n[rule]\n\"sub/file.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, target: &Path| {
        set_prompt_choice(ObstructionChoice::Replace);
        fs::remove_file(target.join("sub/file.txt")).unwrap();
        fs::remove_dir(target.join("sub")).unwrap();
        fs::create_dir(target.join("real")).unwrap();
        symlink(target.join("real"), target.join("sub")).unwrap();
        None
    },
    |_source: &Path, target: &Path| {
        assert!(fs::symlink_metadata(target.join("sub")).unwrap().is_dir());
        assert_eq!(fs::read(target.join("sub/file.txt")).unwrap(), b"hello");
        assert!(fs::symlink_metadata(target.join("real")).unwrap().is_dir());
        assert_eq!(prompt_count(), 1);
    }
    ; "parent_path_replaced_by_symlink_prompts"
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
    |_source: &Path, _target: &Path| "[portal]\n\"missing.txt\" = \"target.txt\"\n"
    => matches Err(e) if e.chain().any(|cause| cause.to_string().contains("literal portal source"))
    ; "missing_literal_source_reports_error"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"same\"\n\"b.txt\" = \"same\"\n"
    }
    => matches Err(e) if e.chain().any(|cause| cause.to_string().contains("collision at"))
    ; "sources_colliding_on_one_target_reports_error"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"dir\"\n\"b.txt\" = \"dir/x\"\n"
    }
    => matches Err(e) if e.chain().any(|cause| cause.to_string().contains("structural conflict"))
    ; "target_used_as_both_file_and_directory_reports_error"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), "{{ missing }}").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"template\" }\n"
    }
    => matches Err(e) if e.chain().any(|cause| cause.to_string().contains("undefined variable"))
    ; "template_with_undefined_variable_reports_error"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        symlink(source.join("missing"), source.join("dangling")).unwrap();
        "[portal]\n\"dangling\" = \"target.txt\"\n"
    }
    => matches Err(e) if e.chain().any(|cause| cause.to_string().contains("not a regular file"))
    ; "dangling_source_symlink_reports_error"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"x").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"symlink\", mode = \"600\" }\n"
    }
    => matches Err(e) if e.chain().any(|cause| cause.to_string().contains("mode"))
    ; "mode_rule_on_symlink_type_reports_error"
)]
#[test_case(
    |source: &Path, target: &Path| {
        fs::write(source.join("file.txt"), b"x").unwrap();
        fs::remove_dir(target).unwrap();
        fs::write(target, b"not a dir").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    }
    => matches Err(e) if e.chain().any(|cause| cause.to_string().contains("is not a directory"))
    ; "target_root_being_a_file_reports_error"
)]
fn apply_error_cases(
    setup: impl Fn(&Path, &Path) -> &'static str,
) -> std::result::Result<dotrift::ExitStatus, miette::Report> {
    ApplyScenario::new(setup).try_run()
}

#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"hello").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    |source: &Path, _target: &Path| {
        fs::remove_file(source.join("file.txt")).unwrap();
        None
    }
    => matches Err(e) if e.chain().any(|cause| cause.to_string().contains("literal portal source"))
    ; "deleting_source_between_runs_reports_error"
)]
fn reapply_error_cases(
    setup: impl Fn(&Path, &Path) -> &'static str,
    modify: impl Fn(&Path, &Path) -> Option<&'static str>,
) -> std::result::Result<dotrift::ExitStatus, miette::Report> {
    let scenario = ApplyScenario::new(setup);
    scenario.run();
    scenario.rewrite(modify);
    scenario.try_run()
}

#[test]
fn creates_missing_target_dir_and_deploys_file() {
    let env = TestEnv::new();
    let source = env.source_dir();
    fs::write(source.join("file.txt"), b"hello").unwrap();
    env.write_config("[portal]\n\"file.txt\" = \"target.txt\"\n");
    let target = env.path("target");
    dotrift::commands::apply::run(&source, Some(target.clone())).unwrap();
    assert!(fs::symlink_metadata(&target).unwrap().is_dir());
    assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"hello");
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
        assert_eq!(
            StateDatabase::open()
                .unwrap()
                .record(&target.join("a.txt"))
                .unwrap(),
            None
        );
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
        assert_eq!(
            StateDatabase::open()
                .unwrap()
                .record(&target.join("a.txt"))
                .unwrap(),
            None
        );
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
        assert_eq!(
            StateDatabase::open()
                .unwrap()
                .record(&target.join("a.txt"))
                .unwrap(),
            None
        );
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
        let database = StateDatabase::open().unwrap();
        assert_eq!(database.record(&target.join("dst/a.txt")).unwrap(), None);
        assert_eq!(database.record(&target.join("dst/sub/b.txt")).unwrap(), None);
    }
    ; "clean_up_removes_every_file_of_removed_directory_portal"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"sub/b.txt\"\n"
    },
    |_source: &Path, _target: &Path| Some("[portal]\n"),
    false,
    |_source: &Path, target: &Path| {
        assert!(fs::symlink_metadata(target.join("a.txt")).is_err());
        assert!(fs::symlink_metadata(target.join("sub/b.txt")).is_err());
        let database = StateDatabase::open().unwrap();
        assert_eq!(database.record(&target.join("a.txt")).unwrap(), None);
        assert_eq!(database.record(&target.join("sub/b.txt")).unwrap(), None);
    }
    ; "clean_up_removes_all_stale_targets"
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
        assert!(
            StateDatabase::open()
                .unwrap()
                .record(&target.join("keep.txt"))
                .unwrap()
                .is_some()
        );
    }
    ; "clean_up_keeps_desired_target_and_removes_stale"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"sub/b.txt\"\n"
    },
    |_source: &Path, _target: &Path| None,
    false,
    |_source: &Path, target: &Path| {
        assert!(target.join("a.txt").is_symlink());
        assert!(target.join("sub/b.txt").is_symlink());
        assert_eq!(fs::read(target.join("a.txt")).unwrap(), b"A");
        assert_eq!(fs::read(target.join("sub/b.txt")).unwrap(), b"B");
        let database = StateDatabase::open().unwrap();
        assert!(database.record(&target.join("a.txt")).unwrap().is_some());
        assert!(database.record(&target.join("sub/b.txt")).unwrap().is_some());
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
        assert_eq!(
            StateDatabase::open()
                .unwrap()
                .record(&target.join("file.txt"))
                .unwrap(),
            None
        );
    }
    ; "clean_up_relinquishes_tampered_target"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"original").unwrap();
        "[portal]\n\"file.txt\" = \"file.txt\"\n[rule]\n\"file.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, target: &Path| {
        fs::remove_file(target.join("file.txt")).unwrap();
        Some("[portal]\n")
    },
    false,
    |_source: &Path, target: &Path| {
        assert!(fs::symlink_metadata(target.join("file.txt")).is_err());
        assert_eq!(
            StateDatabase::open()
                .unwrap()
                .record(&target.join("file.txt"))
                .unwrap(),
            None
        );
    }
    ; "clean_up_relinquishes_missing_target"
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
        symlink(target.join("real"), target.join("sub")).unwrap();
        Some("[portal]\n")
    },
    false,
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("real/file.txt")).unwrap(), b"A");
        assert!(target.join("sub").is_symlink());
        assert_eq!(
            StateDatabase::open()
                .unwrap()
                .record(&target.join("sub/file.txt"))
                .unwrap(),
            None
        );
    }
    ; "clean_up_relinquishes_target_under_symlink_parent"
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
        assert!(fs::symlink_metadata(target.join("a/b/file.txt")).is_err());
        assert_eq!(
            StateDatabase::open()
                .unwrap()
                .record(&target.join("a/b/file.txt"))
                .unwrap(),
            None
        );
    }
    ; "clean_up_prunes_emptied_parent_directories"
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
        assert_eq!(
            StateDatabase::open()
                .unwrap()
                .record(&target.join("a/b/file.txt"))
                .unwrap(),
            None
        );
    }
    ; "clean_up_keeps_empty_directories_without_prune"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"A").unwrap();
        fs::write(source.join("keep.txt"), b"K").unwrap();
        "[portal]\n\"file.txt\" = \"a/b/file.txt\"\n\"keep.txt\" = \"a/keep.txt\"\n"
    },
    |_source: &Path, _target: &Path| Some("[portal]\n\"keep.txt\" = \"a/keep.txt\"\n"),
    true,
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("a/keep.txt")).unwrap(), b"K");
        assert!(fs::metadata(target.join("a")).unwrap().is_dir());
        assert!(fs::symlink_metadata(target.join("a/b")).is_err());
        assert_eq!(
            StateDatabase::open()
                .unwrap()
                .record(&target.join("a/b/file.txt"))
                .unwrap(),
            None
        );
    }
    ; "prune_keeps_parent_holding_remaining_target"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("x.txt"), b"X").unwrap();
        fs::write(source.join("y.txt"), b"Y").unwrap();
        "[portal]\n\"x.txt\" = \"a/x.txt\"\n\"y.txt\" = \"a/y.txt\"\n"
    },
    |_source: &Path, _target: &Path| Some("[portal]\n"),
    true,
    |_source: &Path, target: &Path| {
        assert!(fs::symlink_metadata(target.join("a")).is_err());
        assert!(fs::symlink_metadata(target.join("a/x.txt")).is_err());
        assert!(fs::symlink_metadata(target.join("a/y.txt")).is_err());
        let database = StateDatabase::open().unwrap();
        assert_eq!(database.record(&target.join("a/x.txt")).unwrap(), None);
        assert_eq!(database.record(&target.join("a/y.txt")).unwrap(), None);
    }
    ; "prune_drops_parent_after_last_target_removed"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"b.txt\"\n[rule]\n\"a.txt\" = { type = \"copy\" }\n\"b.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, target: &Path| {
        set_prompt_choice(ObstructionChoice::Skip);
        fs::write(target.join("a.txt"), b"tampered").unwrap();
        Some("[portal]\n\"a.txt\" = \"a.txt\"\n[rule]\n\"a.txt\" = { type = \"copy\" }\n")
    },
    false,
    |_source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("a.txt")).unwrap(), b"tampered");
        assert_eq!(fs::read(target.join("b.txt")).unwrap(), b"B");
        let database = StateDatabase::open().unwrap();
        assert!(database.record(&target.join("a.txt")).unwrap().is_some());
        assert!(database.record(&target.join("b.txt")).unwrap().is_some());
    }
    ; "clean_up_keeps_target_skipped_at_deploy"
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

#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"sub/b.txt\"\n"
    },
    ApplyOptions {
        dry_run: true,
        ..Default::default()
    }
    ; "lists_new_targets_to_deploy"
)]
#[test_case(
    |source: &Path, target: &Path| {
        fs::create_dir_all(target).unwrap();
        fs::write(source.join("file.txt"), b"A").unwrap();
        symlink(source.join("file.txt"), target.join("target.txt")).unwrap();
        StateDatabase::open()
            .unwrap()
            .put(&dotrift::record!(s, target.join("target.txt"), source.join("file.txt")))
            .unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    ApplyOptions {
        dry_run: true,
        ..Default::default()
    }
    ; "labels_replacing_managed_target"
)]
#[test_case(
    |source: &Path, target: &Path| {
        fs::create_dir_all(target).unwrap();
        fs::write(source.join("file.txt"), b"A").unwrap();
        fs::write(target.join("target.txt"), b"unmanaged").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    ApplyOptions {
        dry_run: true,
        ..Default::default()
    }
    ; "labels_unmanaged_target_as_obstruction"
)]
#[test_case(
    |source: &Path, target: &Path| {
        fs::create_dir_all(target).unwrap();
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        fs::write(source.join("c.txt"), b"C").unwrap();
        symlink(source.join("a.txt"), target.join("a.txt")).unwrap();
        StateDatabase::open()
            .unwrap()
            .put(&dotrift::record!(s, target.join("a.txt"), source.join("a.txt")))
            .unwrap();
        fs::write(target.join("c.txt"), b"unmanaged").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"b.txt\"\n\"c.txt\" = \"c.txt\"\n"
    },
    ApplyOptions {
        dry_run: true,
        ..Default::default()
    }
    ; "reports_mixed_target_states_in_sorted_order"
)]
#[test_case(
    |source: &Path, target: &Path| {
        fs::create_dir_all(target).unwrap();
        fs::write(source.join("file.txt"), b"A").unwrap();
        symlink(source.join("file.txt"), target.join("stale.txt")).unwrap();
        StateDatabase::open()
            .unwrap()
            .put(&dotrift::record!(s, target.join("stale.txt"), source.join("file.txt")))
            .unwrap();
        "[portal]\n\"file.txt\" = \"file.txt\"\n"
    },
    ApplyOptions {
        dry_run: true,
        clean_up: true,
        ..Default::default()
    }
    ; "reports_stale_target_removal"
)]
#[test_case(
    |source: &Path, target: &Path| {
        fs::create_dir_all(target.join("a/b")).unwrap();
        fs::write(source.join("file.txt"), b"A").unwrap();
        symlink(source.join("file.txt"), target.join("a/b/stale.txt")).unwrap();
        StateDatabase::open()
            .unwrap()
            .put(&dotrift::record!(s, target.join("a/b/stale.txt"), source.join("file.txt")))
            .unwrap();
        "[portal]\n\"file.txt\" = \"file.txt\"\n"
    },
    ApplyOptions {
        dry_run: true,
        clean_up: true,
        prune_empty_dirs: true,
        ..Default::default()
    }
    ; "reports_emptied_directory_pruning"
)]
#[test_case(
    |source: &Path, target: &Path| {
        fs::create_dir_all(target).unwrap();
        fs::write(source.join("file.txt"), b"A").unwrap();
        fs::write(target.join("a"), b"block").unwrap();
        "[portal]\n\"file.txt\" = \"a/b/file.txt\"\n"
    },
    ApplyOptions {
        dry_run: true,
        ..Default::default()
    }
    ; "labels_parent_directory_obstruction"
)]
#[test_case(
    |source: &Path, target: &Path| {
        fs::create_dir_all(target).unwrap();
        fs::write(source.join("file.txt"), b"A").unwrap();
        fs::write(target.join("target.txt"), b"tampered").unwrap();
        StateDatabase::open()
            .unwrap()
            .put(&dotrift::record!(f, target.join("target.txt"), hash_bytes(b"A")))
            .unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    ApplyOptions {
        dry_run: true,
        ..Default::default()
    }
    ; "labels_tampered_target_as_obstruction"
)]
#[test_case(
    |source: &Path, target: &Path| {
        fs::create_dir_all(target.join("a/b")).unwrap();
        fs::write(source.join("file.txt"), b"A").unwrap();
        fs::write(target.join("a/keep.txt"), b"K").unwrap();
        symlink(source.join("file.txt"), target.join("a/b/stale.txt")).unwrap();
        StateDatabase::open()
            .unwrap()
            .put(&dotrift::record!(s, target.join("a/b/stale.txt"), source.join("file.txt")))
            .unwrap();
        "[portal]\n\"file.txt\" = \"file.txt\"\n"
    },
    ApplyOptions {
        dry_run: true,
        clean_up: true,
        prune_empty_dirs: true,
        ..Default::default()
    }
    ; "keeps_non_empty_parent_out_of_prune_report"
)]
fn dry_run_output(setup: impl Fn(&Path, &Path) -> &'static str, options: ApplyOptions) {
    dotrift::capture::clear();
    let scenario = ApplyScenario::new(setup);
    scenario.run_with_options(options);
    snapshot_settings(&scenario.env).bind(|| {
        insta::assert_snapshot!(test_name(), dotrift::capture::take());
    });
}

#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"sub/b.txt\"\n"
    },
    ApplyOptions {
        verbose: true,
        ..Default::default()
    }
    ; "prints_lines_for_deployed_targets"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"sub/b.txt\"\n"
    },
    ApplyOptions::default()
    ; "default_flags_print_only_summary"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("dotrift_data.toml"), "[variable]\nname = \"x\"\n").unwrap();
        fs::write(source.join("link.txt"), b"L").unwrap();
        fs::write(source.join("copy.txt"), b"C").unwrap();
        fs::write(source.join("tmpl.txt"), "{{ name }}").unwrap();
        "[portal]\n\"link.txt\" = \"link.txt\"\n\"copy.txt\" = \"copy.txt\"\n\"tmpl.txt\" = \"tmpl.txt\"\n[rule]\n\"copy.txt\" = { type = \"copy\" }\n\"tmpl.txt\" = { type = \"template\" }\n"
    },
    ApplyOptions {
        verbose: true,
        ..Default::default()
    }
    ; "reports_symlink_copy_and_template_deploys"
)]
fn verbose_output(setup: impl Fn(&Path, &Path) -> &'static str, options: ApplyOptions) {
    dotrift::capture::clear();
    let scenario = ApplyScenario::new(setup);
    scenario.run_with_options(options);
    snapshot_settings(&scenario.env).bind(|| {
        insta::assert_snapshot!(test_name(), dotrift::capture::take());
    });
}

#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"sub/b.txt\"\n"
    },
    |_source: &Path, _target: &Path| None,
    ApplyOptions {
        verbose: true,
        ..Default::default()
    }
    ; "prints_lines_for_replaced_targets"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n[rule]\n\"a.txt\" = { type = \"copy\" }\n"
    },
    |_source: &Path, target: &Path| {
        set_prompt_choice(ObstructionChoice::Skip);
        fs::write(target.join("a.txt"), b"tampered").unwrap();
        None
    },
    ApplyOptions {
        verbose: true,
        ..Default::default()
    }
    ; "prints_lines_for_skipped_targets"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n[rule]\n\"a.txt\" = { type = \"copy\" }\n"
    },
    |source: &Path, target: &Path| {
        set_prompt_choice(ObstructionChoice::Skip);
        fs::write(source.join("b.txt"), b"B").unwrap();
        fs::write(source.join("c.txt"), b"C").unwrap();
        fs::write(target.join("c.txt"), b"tampered").unwrap();
        Some("[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"b.txt\"\n\"c.txt\" = \"c.txt\"\n[rule]\n\"a.txt\" = { type = \"copy\" }\n\"b.txt\" = { type = \"copy\" }\n\"c.txt\" = { type = \"copy\" }\n")
    },
    ApplyOptions {
        verbose: true,
        ..Default::default()
    }
    ; "reports_mixed_states_in_sorted_order"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"b.txt\"\n"
    },
    |_source: &Path, _target: &Path| Some("[portal]\n\"a.txt\" = \"a.txt\"\n"),
    ApplyOptions {
        verbose: true,
        clean_up: true,
        ..Default::default()
    }
    ; "reports_clean_up_removals"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"A").unwrap();
        "[portal]\n\"file.txt\" = \"a/b/file.txt\"\n"
    },
    |_source: &Path, _target: &Path| Some("[portal]\n"),
    ApplyOptions {
        verbose: true,
        clean_up: true,
        prune_empty_dirs: true,
        ..Default::default()
    }
    ; "reports_clean_up_pruning"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"b.txt\"\n"
    },
    |_source: &Path, _target: &Path| Some("[portal]\n\"a.txt\" = \"a.txt\"\n"),
    ApplyOptions {
        clean_up: true,
        ..Default::default()
    }
    ; "clean_up_prints_nothing_without_verbose"
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
    ApplyOptions {
        verbose: true,
        clean_up: true,
        ..Default::default()
    }
    ; "withholds_relinquished_targets_from_report"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"sub/b.txt\"\n"
    },
    |_source: &Path, _target: &Path| None,
    ApplyOptions {
        verbose: true,
        clean_up: true,
        ..Default::default()
    }
    ; "clean_up_no_op_prints_nothing"
)]
fn verbose_reapply_output(
    setup: impl Fn(&Path, &Path) -> &'static str,
    modify: impl Fn(&Path, &Path) -> Option<&'static str>,
    options: ApplyOptions,
) {
    let scenario = ApplyScenario::new(setup);
    scenario.run();
    scenario.rewrite(modify);
    dotrift::capture::clear();
    scenario.run_with_options(options);
    snapshot_settings(&scenario.env).bind(|| {
        insta::assert_snapshot!(test_name(), dotrift::capture::take());
    });
}

#[test]
fn quiet_flag_suppresses_all_output() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"sub/b.txt\"\n"
    });
    dotrift::capture::clear();
    scenario.run_with_options(ApplyOptions {
        quiet: true,
        ..Default::default()
    });
    assert_eq!(dotrift::capture::take(), "");
}
