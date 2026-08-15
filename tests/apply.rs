mod common;

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use common::TestEnv;
use dotrift::commands::apply::{ObstructionChoice, PROMPT_COUNT, set_prompt_choice};
use dotrift::hash::hash_bytes;
use dotrift::state::{Kind, StateDatabase};
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
