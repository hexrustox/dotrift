mod common;

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use common::TestEnv;
use dotrift::commands::apply::{ApplyOptions, ObstructionChoice, PROMPT_COUNT, set_prompt_choice};
use dotrift::hash::hash_bytes;
use dotrift::state::{Kind, StateDatabase, StateRecord};
use test_case::test_case;

#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"hello").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    |_source: &Path, target: &Path| {
        assert!(target.join("target.txt").is_symlink());
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"hello");
    }
    ; "deploys_portal_file"
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
    ; "deploys_literal_directory_recursively"
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
    ; "deploys_glob_with_stripping_prefix"
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
    ; "copy_rule_overrides_symlink"
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
    ; "template_rule_renders"
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
    ; "mode_rule_applies"
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
    ; "dotrift_toml_renders_from_variable"
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
    ; "active_profile_overrides_base"
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
    ; "stale_active_profile_ignored"
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
    ; "ignores_target"
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
    ; "negation_reincludes"
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
    ; "control_files_implicitly_excluded"
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
    ; "ignore_applies_per_target"
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
    ; "skip_unmanaged_target"
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
    ; "replace_unmanaged_target"
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
    ; "skip_parent_obstruction"
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
    ; "replace_parent_obstruction"
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
        assert_eq!(PROMPT_COUNT.with(|c|*c.borrow()), 1);
    }
    ; "replace_all_latches"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::create_dir_all(source.join("empty")).unwrap();
        "[portal]\n\"empty\" = \"dst\"\n"
    },
    |_source: &Path, _target: &Path| {}
    ; "empty_deployment_succeeds"
)]
fn run_apply_test<F: Fn(&Path, &Path) -> &'static str, G: Fn(&Path, &Path)>(setup: F, assert: G) {
    let env = TestEnv::new();
    let source_dir = env.path("source");
    let target_dir = env.path("target");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&target_dir).unwrap();
    fs::write(
        env.path("source/dotrift.toml"),
        setup(&source_dir, &target_dir),
    )
    .unwrap();
    dotrift::commands::apply::run(&source_dir, Some(target_dir.to_path_buf())).unwrap();
    assert(&source_dir, &target_dir);
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
        assert_eq!(record.link_target, Some(source.join("file.txt")));
    }
    ; "reapply_same_config_is_idempotent"
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
        assert_eq!(record.link_target, Some(source.join("file.txt")));
        assert_eq!(PROMPT_COUNT.with(|count| *count.borrow()), 0);
    }
    ; "symlink_replaces_on_source_change"
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
    ; "copy_replaces_on_source_change"
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
    ; "template_rerenders_on_variable_change"
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
    ; "symlink_rule_changes_to_copy"
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
        assert_eq!(record.link_target, Some(source.join("file.txt")));
    }
    ; "copy_rule_changes_to_symlink"
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
    ; "copy_rule_changes_to_template"
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
    ; "mode_change_reapplies_permissions"
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
        assert_eq!(record.link_target, Some(source.join("new.txt")));
    }
    ; "source_renamed_redirects_symlink"
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
    ; "target_redirected_leaves_stale_path"
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
    ; "entry_removed_leaves_stale_managed_path"
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
    ; "entry_added_deploys_alongside"
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
    ; "glob_widened_deploys_new_file"
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
    ; "ignore_added_makes_path_stale"
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
    ; "profile_activated_between_runs"
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
        assert_eq!(PROMPT_COUNT.with(|count| *count.borrow()), 1);
    }
    ; "tampered_copy_prompt_replace_restores"
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
        assert_eq!(PROMPT_COUNT.with(|count| *count.borrow()), 1);
    }
    ; "tampered_copy_prompt_skip_retains_record"
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
        assert_eq!(record.link_target, Some(source.join("file.txt")));
        assert_eq!(PROMPT_COUNT.with(|count| *count.borrow()), 1);
    }
    ; "tampered_symlink_prompt_replace_restores"
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
    ; "deleted_target_redeploys_despite_stale_record"
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
        assert_eq!(PROMPT_COUNT.with(|count| *count.borrow()), 1);
    }
    ; "parent_dir_replaced_by_symlink_prompts"
)]
fn run_apply_test_twice<
    F: Fn(&Path, &Path) -> &'static str,
    G: Fn(&Path, &Path) -> Option<&'static str>,
    H: Fn(&Path, &Path),
>(
    setup: F,
    modify: G,
    assert: H,
) {
    let env = TestEnv::new();
    let source_dir = env.path("source");
    let target_dir = env.path("target");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&target_dir).unwrap();
    let config = setup(&source_dir, &target_dir);
    fs::write(env.path("source/dotrift.toml"), config).unwrap();
    dotrift::commands::apply::run(&source_dir, Some(target_dir.to_path_buf())).unwrap();
    fs::write(
        env.path("source/dotrift.toml"),
        modify(&source_dir, &target_dir).unwrap_or(config),
    )
    .unwrap();
    dotrift::commands::apply::run(&source_dir, Some(target_dir.to_path_buf())).unwrap();
    assert(&source_dir, &target_dir);
}

#[test_case(
    |_source: &Path, _target: &Path| "[portal]\n\"missing.txt\" = \"target.txt\"\n"
    => matches Err(e) if e.chain().any(|cause| cause.to_string().contains("literal portal source"))
    ; "literal_source_missing_fails"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"same\"\n\"b.txt\" = \"same\"\n"
    }
    => matches Err(e) if e.chain().any(|cause| cause.to_string().contains("collision at"))
    ; "colliding_portals_fail"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"dir\"\n\"b.txt\" = \"dir/x\"\n"
    }
    => matches Err(e) if e.chain().any(|cause| cause.to_string().contains("structural conflict"))
    ; "structural_conflict_fails"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), "{{ missing }}").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"template\" }\n"
    }
    => matches Err(e) if e.chain().any(|cause| cause.to_string().contains("undefined variable"))
    ; "template_undefined_variable_fails"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        symlink(source.join("missing"), source.join("dangling")).unwrap();
        "[portal]\n\"dangling\" = \"target.txt\"\n"
    }
    => matches Err(e) if e.chain().any(|cause| cause.to_string().contains("not a regular file"))
    ; "broken_symlink_literal_source_fails"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("file.txt"), b"x").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n[rule]\n\"target.txt\" = { type = \"symlink\", mode = \"600\" }\n"
    }
    => matches Err(e) if e.chain().any(|cause| cause.to_string().contains("mode"))
    ; "mode_conflicts_with_symlink_rule_fails"
)]
#[test_case(
    |source: &Path, target: &Path| {
        fs::write(source.join("file.txt"), b"x").unwrap();
        fs::remove_dir(target).unwrap();
        fs::write(target, b"not a dir").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    }
    => matches Err(e) if e.chain().any(|cause| cause.to_string().contains("is not a directory"))
    ; "target_directory_is_a_file_fails"
)]
fn run_apply_fails<F: Fn(&Path, &Path) -> &'static str>(
    setup: F,
) -> std::result::Result<dotrift::ExitStatus, miette::Report> {
    let env = TestEnv::new();
    let source_dir = env.path("source");
    let target_dir = env.path("target");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&target_dir).unwrap();
    fs::write(
        env.path("source/dotrift.toml"),
        setup(&source_dir, &target_dir),
    )
    .unwrap();
    dotrift::commands::apply::run(&source_dir, Some(target_dir.to_path_buf()))
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
    ; "source_deleted_between_runs_fails"
)]
fn run_apply_test_twice_fails<
    F: Fn(&Path, &Path) -> &'static str,
    G: Fn(&Path, &Path) -> Option<&'static str>,
>(
    setup: F,
    modify: G,
) -> std::result::Result<dotrift::ExitStatus, miette::Report> {
    let env = TestEnv::new();
    let source_dir = env.path("source");
    let target_dir = env.path("target");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&target_dir).unwrap();
    let config = setup(&source_dir, &target_dir);
    fs::write(env.path("source/dotrift.toml"), config).unwrap();
    dotrift::commands::apply::run(&source_dir, Some(target_dir.to_path_buf())).unwrap();
    fs::write(
        env.path("source/dotrift.toml"),
        modify(&source_dir, &target_dir).unwrap_or(config),
    )
    .unwrap();
    dotrift::commands::apply::run(&source_dir, Some(target_dir.to_path_buf()))
}

#[test]
fn creates_missing_target_directory() {
    let env = TestEnv::new();
    let source_dir = env.path("source");
    let target_dir = env.path("target");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join("file.txt"), b"hello").unwrap();
    fs::write(
        source_dir.join("dotrift.toml"),
        "[portal]\n\"file.txt\" = \"target.txt\"\n",
    )
    .unwrap();
    dotrift::commands::apply::run(&source_dir, Some(target_dir.clone())).unwrap();
    assert!(fs::symlink_metadata(&target_dir).unwrap().is_dir());
    assert_eq!(fs::read(target_dir.join("target.txt")).unwrap(), b"hello");
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
    ; "removes_stale_symlink"
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
    ; "removes_stale_copy"
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
    ; "removes_stale_template"
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
    ; "removes_stale_directory_portal"
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
    ; "removes_multiple_stale_targets"
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
    ; "keeps_desired_and_removes_stale"
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
    ; "no_op_when_nothing_stale"
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
    ; "relinquishes_tampered_stale"
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
    ; "relinquishes_missing_stale"
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
    ; "relinquishes_path_under_symlink_parent"
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
    ; "prunes_empty_parent_dirs"
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
    ; "leaves_empty_dirs_without_prune_flag"
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
    ; "prune_keeps_non_empty_parent"
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
    ; "prunes_parent_after_last_removal"
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
    ; "clean_up_skipped_when_entry_skipped"
)]
fn test_apply_clean_up<
    F: Fn(&Path, &Path) -> &'static str,
    G: Fn(&Path, &Path) -> Option<&'static str>,
    H: Fn(&Path, &Path),
>(
    setup: F,
    modify: G,
    prune: bool,
    assert: H,
) {
    let env = TestEnv::new();
    let source_dir = env.path("source");
    let target_dir = env.path("target");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&target_dir).unwrap();
    let config = setup(&source_dir, &target_dir);
    fs::write(env.path("source/dotrift.toml"), config).unwrap();
    dotrift::commands::apply::run(&source_dir, Some(target_dir.to_path_buf())).unwrap();
    fs::write(
        env.path("source/dotrift.toml"),
        modify(&source_dir, &target_dir).unwrap_or(config),
    )
    .unwrap();
    dotrift::commands::apply::run_with_options(
        &source_dir,
        Some(target_dir.to_path_buf()),
        ApplyOptions {
            clean_up: true,
            prune_empty_dirs: prune,
            ..Default::default()
        },
    )
    .unwrap();
    assert(&source_dir, &target_dir);
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
    ; "deploys_lists_new_targets"
)]
#[test_case(
    |source: &Path, target: &Path| {
        fs::create_dir_all(target).unwrap();
        fs::write(source.join("file.txt"), b"A").unwrap();
        symlink(source.join("file.txt"), target.join("target.txt")).unwrap();
        StateDatabase::open()
            .unwrap()
            .put(&StateRecord {
                target_path: target.join("target.txt"),
                source_path: source.join("file.txt"),
                kind: Kind::Symlink,
                link_target: Some(source.join("file.txt")),
                content_hash: None,
            })
            .unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    ApplyOptions {
        dry_run: true,
        ..Default::default()
    }
    ; "labels_managed_target_replaced"
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
    ; "labels_unmanaged_target_obstruction"
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
            .put(&StateRecord {
                target_path: target.join("a.txt"),
                source_path: source.join("a.txt"),
                kind: Kind::Symlink,
                link_target: Some(source.join("a.txt")),
                content_hash: None,
            })
            .unwrap();
        fs::write(target.join("c.txt"), b"unmanaged").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"b.txt\"\n\"c.txt\" = \"c.txt\"\n"
    },
    ApplyOptions {
        dry_run: true,
        ..Default::default()
    }
    ; "mixed_states_sorted"
)]
#[test_case(
    |source: &Path, target: &Path| {
        fs::create_dir_all(target).unwrap();
        fs::write(source.join("file.txt"), b"A").unwrap();
        symlink(source.join("file.txt"), target.join("stale.txt")).unwrap();
        StateDatabase::open()
            .unwrap()
            .put(&StateRecord {
                target_path: target.join("stale.txt"),
                source_path: source.join("file.txt"),
                kind: Kind::Symlink,
                link_target: Some(source.join("file.txt")),
                content_hash: None,
            })
            .unwrap();
        "[portal]\n\"file.txt\" = \"file.txt\"\n"
    },
    ApplyOptions {
        dry_run: true,
        clean_up: true,
        ..Default::default()
    }
    ; "clean_up_reports_removal"
)]
#[test_case(
    |source: &Path, target: &Path| {
        fs::create_dir_all(target.join("a/b")).unwrap();
        fs::write(source.join("file.txt"), b"A").unwrap();
        symlink(source.join("file.txt"), target.join("a/b/stale.txt")).unwrap();
        StateDatabase::open()
            .unwrap()
            .put(&StateRecord {
                target_path: target.join("a/b/stale.txt"),
                source_path: source.join("file.txt"),
                kind: Kind::Symlink,
                link_target: Some(source.join("file.txt")),
                content_hash: None,
            })
            .unwrap();
        "[portal]\n\"file.txt\" = \"file.txt\"\n"
    },
    ApplyOptions {
        dry_run: true,
        clean_up: true,
        prune_empty_dirs: true,
        ..Default::default()
    }
    ; "clean_up_prune_empty_dirs_reports_pruning"
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
    ; "labels_parent_obstruction"
)]
#[test_case(
    |source: &Path, target: &Path| {
        fs::create_dir_all(target).unwrap();
        fs::write(source.join("file.txt"), b"A").unwrap();
        fs::write(target.join("target.txt"), b"tampered").unwrap();
        StateDatabase::open()
            .unwrap()
            .put(&StateRecord {
                target_path: target.join("target.txt"),
                source_path: source.join("file.txt"),
                kind: Kind::File,
                link_target: None,
                content_hash: Some(hash_bytes(b"A")),
            })
            .unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    ApplyOptions {
        dry_run: true,
        ..Default::default()
    }
    ; "labels_tampered_managed_target_obstruction"
)]
#[test_case(
    |source: &Path, target: &Path| {
        fs::create_dir_all(target.join("a/b")).unwrap();
        fs::write(source.join("file.txt"), b"A").unwrap();
        fs::write(target.join("a/keep.txt"), b"K").unwrap();
        symlink(source.join("file.txt"), target.join("a/b/stale.txt")).unwrap();
        StateDatabase::open()
            .unwrap()
            .put(&StateRecord {
                target_path: target.join("a/b/stale.txt"),
                source_path: source.join("file.txt"),
                kind: Kind::Symlink,
                link_target: Some(source.join("file.txt")),
                content_hash: None,
            })
            .unwrap();
        "[portal]\n\"file.txt\" = \"file.txt\"\n"
    },
    ApplyOptions {
        dry_run: true,
        clean_up: true,
        prune_empty_dirs: true,
        ..Default::default()
    }
    ; "clean_up_prune_skips_non_empty_parent"
)]
fn test_dry_run<F: Fn(&Path, &Path) -> &'static str>(setup: F, options: ApplyOptions) {
    dotrift::capture::clear();
    let env = TestEnv::new();
    let source_dir = env.path("source");
    let target_dir = env.path("target");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(
        env.path("source/dotrift.toml"),
        setup(&source_dir, &target_dir),
    )
    .unwrap();
    dotrift::commands::apply::run_with_options(&source_dir, Some(target_dir.clone()), options)
        .unwrap();

    let test_name = std::thread::current().name().unwrap().replace(":", "_");
    let captured = dotrift::capture::take();
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(env.root().to_str().unwrap(), "<root>");
    settings.bind(|| {
        insta::assert_snapshot!(test_name, captured);
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
    ; "deployed_lines"
)]
#[test_case(
    |source: &Path, _target: &Path| {
        fs::write(source.join("a.txt"), b"A").unwrap();
        fs::write(source.join("b.txt"), b"B").unwrap();
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"sub/b.txt\"\n"
    },
    ApplyOptions::default()
    ; "default_prints_only_summary"
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
    ; "mixed_deploy_types_deployed"
)]
fn test_apply_verbose<F: Fn(&Path, &Path) -> &'static str>(setup: F, options: ApplyOptions) {
    dotrift::capture::clear();
    let env = TestEnv::new();
    let source_dir = env.path("source");
    let target_dir = env.path("target");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&target_dir).unwrap();
    fs::write(
        env.path("source/dotrift.toml"),
        setup(&source_dir, &target_dir),
    )
    .unwrap();
    dotrift::commands::apply::run_with_options(&source_dir, Some(target_dir.clone()), options)
        .unwrap();
    let test_name = std::thread::current().name().unwrap().replace(":", "_");
    let captured = dotrift::capture::take();
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(env.root().to_str().unwrap(), "<root>");
    settings.bind(|| {
        insta::assert_snapshot!(test_name, captured);
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
    ; "replaced_lines"
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
    ; "skipped_lines"
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
    ; "mixed_walk_sorted"
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
    ; "clean_up_removed_lines"
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
    ; "clean_up_pruned_lines"
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
    ; "clean_up_silent_without_verbose"
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
    ; "relinquished_not_printed"
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
    ; "no_op_clean_up"
)]
fn test_apply_verbose_twice<
    F: Fn(&Path, &Path) -> &'static str,
    G: Fn(&Path, &Path) -> Option<&'static str>,
>(
    setup: F,
    modify: G,
    options: ApplyOptions,
) {
    let env = TestEnv::new();
    let source_dir = env.path("source");
    let target_dir = env.path("target");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&target_dir).unwrap();
    let config = setup(&source_dir, &target_dir);
    fs::write(env.path("source/dotrift.toml"), config).unwrap();
    dotrift::commands::apply::run(&source_dir, Some(target_dir.to_path_buf())).unwrap();
    fs::write(
        env.path("source/dotrift.toml"),
        modify(&source_dir, &target_dir).unwrap_or(config),
    )
    .unwrap();
    dotrift::capture::clear();
    dotrift::commands::apply::run_with_options(&source_dir, Some(target_dir.to_path_buf()), options)
        .unwrap();
    let test_name = std::thread::current().name().unwrap().replace(":", "_");
    let captured = dotrift::capture::take();
    let mut settings = insta::Settings::clone_current();
    settings.add_filter(env.root().to_str().unwrap(), "<root>");
    settings.bind(|| {
        insta::assert_snapshot!(test_name, captured);
    });
}

#[test]
fn quiet_suppresses_summary() {
    let env = TestEnv::new();
    let source_dir = env.path("source");
    let target_dir = env.path("target");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&target_dir).unwrap();
    fs::write(source_dir.join("a.txt"), b"A").unwrap();
    fs::write(source_dir.join("b.txt"), b"B").unwrap();
    fs::write(
        source_dir.join("dotrift.toml"),
        "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"sub/b.txt\"\n",
    )
    .unwrap();
    dotrift::capture::clear();
    dotrift::commands::apply::run_with_options(
        &source_dir,
        Some(target_dir.clone()),
        ApplyOptions {
            quiet: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(dotrift::capture::take(), "");
}
