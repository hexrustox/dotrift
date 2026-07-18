use std::{
    fs,
    path::{Path, PathBuf},
};

use dotrift::cli::GlobalFlags;
use tempfile::TempDir;

pub fn setup_test(config: &str) -> (TempDir, PathBuf, PathBuf, PathBuf) {
    let temp_dir = tempfile::tempdir().unwrap();
    let source_dir = temp_dir.path().join("source");
    let target_dir = temp_dir.path().join("target");
    let db_path = temp_dir.path().join("db");

    fs::create_dir(&source_dir).unwrap();
    fs::create_dir(&target_dir).unwrap();
    fs::write(source_dir.join("dotrift.toml"), config).unwrap();
    fs::write(source_dir.join("a.txt"), "").unwrap();
    fs::write(source_dir.join("b.txt"), "").unwrap();
    fs::create_dir(source_dir.join("subdir")).unwrap();
    fs::write(source_dir.join("subdir").join("c.txt"), "").unwrap();
    fs::write(source_dir.join("subdir").join("d.txt"), "").unwrap();

    (temp_dir, source_dir, target_dir, db_path)
}

#[allow(dead_code)]
pub fn flags(source_dir: &Path, target_dir: &Path) -> GlobalFlags {
    GlobalFlags::new(
        Some(source_dir.to_path_buf()),
        Some(target_dir.to_path_buf()),
        None,
    )
}
