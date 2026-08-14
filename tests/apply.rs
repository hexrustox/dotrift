mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use common::TestEnv;
use dotrift::commands::apply::{
    ApplyOptions, ObstructionChoice, PROMPT_COUNT, run_with_options, set_prompt_choice,
};
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
    |source: &Path, target: &Path| {
        fs::write(target.join("target.txt"), b"old").unwrap();
        StateDatabase::open()
            .unwrap()
            .put(&StateRecord {
                target_path: target.join("target.txt"),
                source_path: source.join("file.txt"),
                kind: Kind::File,
                link_target: None,
                content_hash: Some(hash_bytes(b"old")),
            })
            .unwrap();
        fs::write(source.join("file.txt"), b"new").unwrap();
        "[portal]\n\"file.txt\" = \"target.txt\"\n"
    },
    |source: &Path, target: &Path| {
        assert_eq!(fs::read(target.join("target.txt")).unwrap(), b"new");
        let record = StateDatabase::open()
            .unwrap()
            .record(&target.join("target.txt"))
            .unwrap()
            .unwrap();
        assert_eq!(record.kind, Kind::Symlink);
        assert_eq!(record.link_target, Some(source.join("file.txt")));
    }
    ; "managed_path_auto_replaced"
)]
#[test_case(
    |source: &Path, target: &Path| {
        set_prompt_choice(ObstructionChoice::Replace);
        fs::write(target.join("target.txt"), b"tampered").unwrap();
        StateDatabase::open()
            .unwrap()
            .put(&StateRecord {
                target_path: target.join("target.txt"),
                source_path: source.join("file.txt"),
                kind: Kind::File,
                link_target: None,
                content_hash: Some(hash_bytes(b"old")),
            })
            .unwrap();
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
    ; "modified_managed_path_prompts"
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

#[test]
fn clean_up_removes_managed_paths_excluded_from_desired_deployment() {
    let env = TestEnv::new();
    let source = env.path("source");
    let target = env.path("target");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("file.txt"), b"hello").unwrap();
    fs::write(
        source.join("dotrift.toml"),
        "[portal]\n\"file.txt\" = \"old.txt\"\n",
    )
    .unwrap();
    run_with_options(&source, Some(target.clone()), ApplyOptions::default()).unwrap();
    fs::write(source.join(".dotriftignore"), "old.txt\n").unwrap();
    run_with_options(
        &source,
        Some(target.clone()),
        ApplyOptions {
            clean_up: true,
            ..ApplyOptions::default()
        },
    )
    .unwrap();
    assert!(!target.join("old.txt").exists());
    assert!(
        env.database()
            .record(&target.join("old.txt"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn clean_up_relinquishes_modified_and_missing_stale_paths() {
    let env = TestEnv::new();
    let source = env.path("source");
    let target = env.path("target");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("file.txt"), b"hello").unwrap();
    fs::write(
        source.join("dotrift.toml"),
        "[portal]\n\"file.txt\" = \"modified.txt\"\n\"file.txt\" = \"missing.txt\"\n",
    )
    .unwrap();
    run_with_options(&source, Some(target.clone()), ApplyOptions::default()).unwrap_err();
    fs::write(
        source.join("dotrift.toml"),
        "[portal]\n\"file.txt\" = \"modified.txt\"\n",
    )
    .unwrap();
    run_with_options(&source, Some(target.clone()), ApplyOptions::default()).unwrap();
    fs::remove_file(target.join("modified.txt")).unwrap();
    fs::write(target.join("modified.txt"), b"changed").unwrap();
    fs::write(source.join(".dotriftignore"), "modified.txt\n").unwrap();
    run_with_options(
        &source,
        Some(target.clone()),
        ApplyOptions {
            clean_up: true,
            ..ApplyOptions::default()
        },
    )
    .unwrap();
    assert_eq!(fs::read(target.join("modified.txt")).unwrap(), b"changed");
    assert!(
        env.database()
            .record(&target.join("modified.txt"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn clean_up_prunes_empty_parent_directories_but_not_target_root() {
    let env = TestEnv::new();
    let source = env.path("source");
    let target = env.path("target");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("file.txt"), b"hello").unwrap();
    fs::write(
        source.join("dotrift.toml"),
        "[portal]\n\"file.txt\" = \"nested/old.txt\"\n",
    )
    .unwrap();
    run_with_options(&source, Some(target.clone()), ApplyOptions::default()).unwrap();
    fs::write(source.join(".dotriftignore"), "nested/old.txt\n").unwrap();
    run_with_options(
        &source,
        Some(target.clone()),
        ApplyOptions {
            clean_up: true,
            prune_empty_dirs: true,
            ..ApplyOptions::default()
        },
    )
    .unwrap();
    assert!(target.is_dir());
    assert!(!target.join("nested").exists());
}

#[test]
fn clean_up_does_not_follow_symlink_parents() {
    let env = TestEnv::new();
    let source = env.path("source");
    let target = env.path("target");
    let outside = env.path("outside");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("old.txt"), b"keep").unwrap();
    fs::create_dir_all(&target).unwrap();
    std::os::unix::fs::symlink(&outside, target.join("link")).unwrap();
    fs::write(source.join("dotrift.toml"), "[portal]\n").unwrap();
    env.database()
        .put(&StateRecord {
            target_path: target.join("link/old.txt"),
            source_path: source.join("old.txt"),
            kind: Kind::File,
            link_target: None,
            content_hash: Some(hash_bytes(b"keep")),
        })
        .unwrap();
    run_with_options(
        &source,
        Some(target.clone()),
        ApplyOptions {
            clean_up: true,
            prune_empty_dirs: true,
            ..ApplyOptions::default()
        },
    )
    .unwrap();
    assert_eq!(fs::read(outside.join("old.txt")).unwrap(), b"keep");
    assert!(
        env.database()
            .record(&target.join("link/old.txt"))
            .unwrap()
            .is_none()
    );
}
