mod common;

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;

use common::{ApplyScenario, Prompt, record_of};
use dotrift::commands::apply::ApplyOptions;
use dotrift::state::{Kind, hash_bytes};

#[test]
fn first_apply_deploys_single_file_as_symlink() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });

    scenario.run();

    let link = scenario.target.join("file1");
    let metadata = fs::symlink_metadata(&link).unwrap();
    assert!(metadata.file_type().is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), scenario.source.join("file1"));
    let record = record_of(&scenario.env, &link).expect("no state record for deployed symlink");
    assert_eq!(record.source_path, scenario.source.join("file1"));
    assert_eq!(record.kind, Kind::Symlink);
    assert_eq!(record.content_hash, None);
}

#[test]
fn symlinked_source_deploys_under_logical_path() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        symlink(source.join("file1"), source.join("link1")).unwrap();
        r#"
[portal]
"link1" = "link1"
"#
    });

    scenario.run();

    let link = scenario.target.join("link1");
    let metadata = fs::symlink_metadata(&link).unwrap();
    assert!(metadata.file_type().is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), scenario.source.join("link1"));
    let record = record_of(&scenario.env, &link).expect("no state record for deployed symlink");
    assert_eq!(record.source_path, scenario.source.join("link1"));
    assert_eq!(record.kind, Kind::Symlink);
}

#[test]
fn copy_rule_resolves_symlinked_source() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        symlink(source.join("file1"), source.join("link1")).unwrap();
        r#"
[portal]
"link1" = "link1"

[rule]
"link1" = { type = "copy" }
"#
    });

    scenario.run();

    let target = scenario.target.join("link1");
    let metadata = fs::symlink_metadata(&target).unwrap();
    assert!(metadata.file_type().is_file());
    assert_eq!(fs::read(&target).unwrap(), b"content1");
    let record = record_of(&scenario.env, &target).expect("no state record for deployed copy");
    assert_eq!(record.kind, Kind::File);
    assert_eq!(
        record.content_hash.as_deref(),
        Some(hash_bytes(b"content1").as_str())
    );
}

#[test]
fn literal_dir_portal_deploys_nested_tree() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::create_dir_all(source.join("dir1/sub1")).unwrap();
        fs::write(source.join("dir1/file1"), "content1").unwrap();
        fs::write(source.join("dir1/sub1/file2"), "content2").unwrap();
        r#"
[portal]
"dir1" = "dir1"
"#
    });

    scenario.run();

    let file1 = scenario.target.join("dir1/file1");
    let file2 = scenario.target.join("dir1/sub1/file2");
    for (target, content, relative) in [
        (&file1, "content1", Path::new("dir1/file1")),
        (&file2, "content2", Path::new("dir1/sub1/file2")),
    ] {
        let metadata = fs::symlink_metadata(target).unwrap();
        assert!(metadata.file_type().is_symlink());
        assert_eq!(
            fs::read_link(target).unwrap(),
            scenario.source.join(relative)
        );
        let record =
            record_of(&scenario.env, target).expect("no state record for deployed symlink");
        assert_eq!(record.source_path, scenario.source.join(relative));
        assert_eq!(record.kind, Kind::Symlink);
        assert_eq!(record.content_hash, None);
        assert_eq!(fs::read(target).unwrap(), content.as_bytes());
    }
}

#[test]
fn symlinked_dir_children_use_logical_paths() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::create_dir_all(source.join("dir1")).unwrap();
        fs::write(source.join("dir1/file1"), "content1").unwrap();
        symlink(source.join("dir1"), source.join("link1")).unwrap();
        r#"
[portal]
"link1" = "dir1"
"#
    });

    scenario.run();

    let link = scenario.target.join("dir1/file1");
    let metadata = fs::symlink_metadata(&link).unwrap();
    assert!(metadata.file_type().is_symlink());
    assert_eq!(
        fs::read_link(&link).unwrap(),
        scenario.source.join("link1/file1")
    );
    let record = record_of(&scenario.env, &link).expect("no state record for deployed symlink");
    assert_eq!(record.source_path, scenario.source.join("link1/file1"));
    assert_eq!(record.kind, Kind::Symlink);
}

#[test]
fn copy_deploy_records_content_hash() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "copy" }
"#
    });

    scenario.run();

    let target = scenario.target.join("file1");
    let metadata = fs::symlink_metadata(&target).unwrap();
    assert!(metadata.file_type().is_file());
    assert_eq!(fs::read(&target).unwrap(), b"content1");
    let record = record_of(&scenario.env, &target).expect("no state record for deployed copy");
    assert_eq!(record.kind, Kind::File);
    assert_eq!(
        record.content_hash.as_deref(),
        Some(hash_bytes(b"content1").as_str())
    );
}

#[test]
fn template_renders_data_variables() {
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

    scenario.run();

    let target = scenario.target.join("file1");
    let metadata = fs::symlink_metadata(&target).unwrap();
    assert!(metadata.file_type().is_file());
    assert_eq!(fs::read(&target).unwrap(), b"str\n");
    let record = record_of(&scenario.env, &target).expect("no state record for deployed template");
    assert_eq!(record.kind, Kind::File);
    assert_eq!(
        record.content_hash.as_deref(),
        Some(hash_bytes(b"str\n").as_str())
    );
}

#[test]
fn mode_rule_sets_permissions_on_copy() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "copy", mode = "600" }
"#
    });

    scenario.run();

    let target = scenario.target.join("file1");
    let metadata = fs::symlink_metadata(&target).unwrap();
    assert!(metadata.file_type().is_file());
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    assert_eq!(fs::read(&target).unwrap(), b"content1");
}

#[test]
fn templated_portal_key_resolves_from_variable() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"{{ file1 }}" = "file1"
"#
    });
    scenario
        .env
        .write_data_file("[variable]\nfile1 = \"file1\"\n");

    scenario.run();

    let link = scenario.target.join("file1");
    let metadata = fs::symlink_metadata(&link).unwrap();
    assert!(metadata.file_type().is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), scenario.source.join("file1"));
}

#[test]
fn empty_dir_portal_deploys_nothing() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::create_dir(source.join("dir1")).unwrap();
        r#"
[portal]
"dir1" = "dir1"
"#
    });

    scenario.run();

    assert!(!scenario.target.join("dir1").exists());
    assert!(scenario.env.database().managed_paths().unwrap().is_empty());
}

#[test]
fn missing_target_root_is_created() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });
    fs::remove_dir_all(&scenario.target).unwrap();

    scenario.run();

    let link = scenario.target.join("file1");
    let metadata = fs::symlink_metadata(&link).unwrap();
    assert!(metadata.file_type().is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), scenario.source.join("file1"));
    let record = record_of(&scenario.env, &link).expect("no state record for deployed symlink");
    assert_eq!(record.kind, Kind::Symlink);
}

#[test]
fn reapply_rewires_symlink_after_source_change() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });

    scenario.run();
    fs::write(scenario.source.join("file1"), "content2").unwrap();
    scenario.run_with(ApplyOptions::default(), &Prompt::never());

    let link = scenario.target.join("file1");
    let metadata = fs::symlink_metadata(&link).unwrap();
    assert!(metadata.file_type().is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), scenario.source.join("file1"));
    assert_eq!(fs::read(&link).unwrap(), b"content2");
    let record = record_of(&scenario.env, &link).expect("no state record after rewire");
    assert_eq!(record.source_path, scenario.source.join("file1"));
    assert_eq!(record.kind, Kind::Symlink);
}

#[test]
fn reapply_redeploys_copy_and_updates_hash() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "copy" }
"#
    });

    scenario.run();
    fs::write(scenario.source.join("file1"), "content2").unwrap();
    scenario.run();

    let target = scenario.target.join("file1");
    let metadata = fs::symlink_metadata(&target).unwrap();
    assert!(metadata.file_type().is_file());
    assert_eq!(fs::read(&target).unwrap(), b"content2");
    let record = record_of(&scenario.env, &target).expect("no state record after redeploy");
    assert_eq!(
        record.content_hash.as_deref(),
        Some(hash_bytes(b"content2").as_str())
    );
}

#[test]
fn reapply_rerenders_template_on_variable_change() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "{{ str }}\n").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "template" }
"#
    });
    scenario
        .env
        .write_data_file("[variable]\nstr = \"content1\"\n");

    scenario.run();
    scenario
        .env
        .write_data_file("[variable]\nstr = \"content2\"\n");
    scenario.run();

    let target = scenario.target.join("file1");
    let metadata = fs::symlink_metadata(&target).unwrap();
    assert!(metadata.file_type().is_file());
    assert_eq!(fs::read(&target).unwrap(), b"content2\n");
    let record = record_of(&scenario.env, &target).expect("no state record after re-render");
    assert_eq!(
        record.content_hash.as_deref(),
        Some(hash_bytes(b"content2\n").as_str())
    );
}

#[test]
fn adding_copy_rule_converts_symlink_to_file() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });

    scenario.run();
    scenario.write_config(
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "copy" }
"#,
    );
    scenario.run();

    let target = scenario.target.join("file1");
    let metadata = fs::symlink_metadata(&target).unwrap();
    assert!(metadata.file_type().is_file());
    assert_eq!(fs::read(&target).unwrap(), b"content1");
    let record = record_of(&scenario.env, &target).expect("no state record after conversion");
    assert_eq!(record.kind, Kind::File);
    assert_eq!(
        record.content_hash.as_deref(),
        Some(hash_bytes(b"content1").as_str())
    );
}

#[test]
fn removing_copy_rule_reverts_file_to_symlink() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "copy" }
"#
    });

    scenario.run();
    scenario.write_config(
        r#"
[portal]
"file1" = "file1"
"#,
    );
    scenario.run();

    let link = scenario.target.join("file1");
    let metadata = fs::symlink_metadata(&link).unwrap();
    assert!(metadata.file_type().is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), scenario.source.join("file1"));
    let record = record_of(&scenario.env, &link).expect("no state record after revert");
    assert_eq!(record.kind, Kind::Symlink);
    assert_eq!(record.content_hash, None);
}

#[test]
fn mode_change_updates_permissions_on_copy() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "copy", mode = "600" }
"#
    });

    scenario.run();
    scenario.write_config(
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "copy", mode = "644" }
"#,
    );
    scenario.run();

    let target = scenario.target.join("file1");
    let metadata = fs::symlink_metadata(&target).unwrap();
    assert!(metadata.file_type().is_file());
    assert_eq!(metadata.permissions().mode() & 0o777, 0o644);
    assert_eq!(fs::read(&target).unwrap(), b"content1");
}

#[test]
fn source_rename_redirects_record() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "content1").unwrap();
        r#"
[portal]
"file1" = "file1"
"#
    });

    scenario.run();
    fs::rename(scenario.source.join("file1"), scenario.source.join("file2")).unwrap();
    scenario.write_config(
        r#"
[portal]
"file2" = "file1"
"#,
    );
    scenario.run();

    let link = scenario.target.join("file1");
    let metadata = fs::symlink_metadata(&link).unwrap();
    assert!(metadata.file_type().is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), scenario.source.join("file2"));
    let record = record_of(&scenario.env, &link).expect("no state record after rename");
    assert_eq!(record.source_path, scenario.source.join("file2"));
    assert_eq!(record.kind, Kind::Symlink);
}

#[test]
fn profile_override_rerenders_template() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "{{ str }}\n").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "template" }
"#
    });
    scenario.env.write_data_file(
        "[variable]\nstr = \"content1\"\n\n[profile.profile1]\nstr = \"content2\"\n",
    );

    scenario.run();
    scenario
        .env
        .database()
        .activate_profile("profile1")
        .unwrap();
    scenario.run();

    let target = scenario.target.join("file1");
    let metadata = fs::symlink_metadata(&target).unwrap();
    assert!(metadata.file_type().is_file());
    assert_eq!(fs::read(&target).unwrap(), b"content2\n");
    let record =
        record_of(&scenario.env, &target).expect("no state record after profile re-render");
    assert_eq!(
        record.content_hash.as_deref(),
        Some(hash_bytes(b"content2\n").as_str())
    );
}

#[test]
fn copy_with_mode_and_template_combined() {
    let scenario = ApplyScenario::new(|source, _target| {
        fs::write(source.join("file1"), "{{ str }}\n").unwrap();
        r#"
[portal]
"file1" = "file1"

[rule]
"file1" = { type = "template", mode = "640" }
"#
    });
    scenario.env.write_data_file("[variable]\nstr = \"str\"\n");

    scenario.run();

    let target = scenario.target.join("file1");
    let metadata = fs::symlink_metadata(&target).unwrap();
    assert!(metadata.file_type().is_file());
    assert_eq!(fs::read(&target).unwrap(), b"str\n");
    assert_eq!(metadata.permissions().mode() & 0o777, 0o640);
    let record = record_of(&scenario.env, &target).expect("no state record for deployed template");
    assert_eq!(
        record.content_hash.as_deref(),
        Some(hash_bytes(b"str\n").as_str())
    );
}
