use std::fs;

use dotrift::cli::{GlobalFlags, TemplaterFlags};
use dotrift::command::templater;

fn setup() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    let db_path = temp_dir.path().join("db");
    let out_path = temp_dir.path().join("output");

    fs::create_dir(&source_dir).unwrap();

    (temp_dir, source_dir, db_path, out_path)
}

fn flags(source_dir: &std::path::Path) -> GlobalFlags {
    GlobalFlags::new(Some(source_dir.to_path_buf()), None, None)
}

#[test]
fn test_data_file_and_var_override() {
    let (_tmp, source_dir, db_path, out_path) = setup();
    fs::write(
        source_dir.join("dotrift_data.toml"),
        r#"[variable]
name = "from_data""#,
    )
    .unwrap();

    templater::run(
        flags(&source_dir),
        &db_path,
        TemplaterFlags {
            string: Some("{{ name }}".into()),
            file: None,
            output: Some(out_path.clone()),
            var: vec!["name=\"from_cli\"".into()],
            no_data: false,
            data_path: None,
        },
    )
    .unwrap();

    assert_eq!(fs::read_to_string(&out_path).unwrap(), "from_cli");
}

#[test]
fn test_file_template_with_builtins() {
    let (_tmp, source_dir, db_path, out_path) = setup();
    let template_path = source_dir.join("template.txt");
    fs::write(
        &template_path,
        "{% for x in items %}{{ upper(x) }}{% end %}",
    )
    .unwrap();

    templater::run(
        flags(&source_dir),
        &db_path,
        TemplaterFlags {
            string: None,
            file: Some(template_path),
            output: Some(out_path.clone()),
            var: vec!["items=[\"a\",\"b\",\"c\"]".into()],
            no_data: true,
            data_path: None,
        },
    )
    .unwrap();

    assert_eq!(fs::read_to_string(&out_path).unwrap(), "ABC");
}

#[test]
fn test_no_data_ignores_dotrift_data() {
    let (_tmp, source_dir, db_path, out_path) = setup();
    fs::write(
        source_dir.join("dotrift_data.toml"),
        r#"[variable]
name = "from_data""#,
    )
    .unwrap();

    templater::run(
        flags(&source_dir),
        &db_path,
        TemplaterFlags {
            string: Some("{{ name }}".into()),
            file: None,
            output: Some(out_path.clone()),
            var: vec!["name=\"from_cli\"".into()],
            no_data: true,
            data_path: None,
        },
    )
    .unwrap();

    assert_eq!(fs::read_to_string(&out_path).unwrap(), "from_cli");
}
