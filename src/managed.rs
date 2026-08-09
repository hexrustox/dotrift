use std::fs;

use miette::Result;

use crate::hash;
use crate::state::{Kind, StateRecord};

pub fn is_managed(record: &StateRecord) -> Result<bool> {
    let metadata = match fs::symlink_metadata(&record.target_path) {
        Ok(metadata) => metadata,
        Err(_) => return Ok(false),
    };

    match record.kind {
        Kind::Symlink => {
            if !metadata.file_type().is_symlink() {
                return Ok(false);
            }
            Ok(fs::read_link(&record.target_path).ok().as_ref() == record.link_target.as_ref())
        }
        Kind::File => {
            if !metadata.file_type().is_file() {
                return Ok(false);
            }
            match hash::hash_file(&record.target_path) {
                Ok(hash) => Ok(Some(hash) == record.content_hash),
                Err(_) => Ok(false),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;
    use test_case::test_case;

    use crate::hash::hash_bytes;

    use super::*;

    #[test_case(
        |t| fs::write(t.join("file"), "hello").unwrap(),
        |t| crate::record!(f, t.join("file"), "", hash_bytes(b"hello")) => true;
        "file_matches_content"
    )]
    #[test_case(
        |t| fs::write(t.join("file"), "hello").unwrap(),
        |t| crate::record!(f, t.join("file"), "", hash_bytes(b"world")) => false;
        "file_content_differs_from_record"
    )]
    #[test_case(
        |_| {},
        |t| crate::record!(f, t.join("file"), "", hash_bytes(b"hello")) => false;
        "file_record_but_target_missing"
    )]
    #[test_case(
        |t| std::os::unix::fs::symlink(t.join("elsewhere"), t.join("file")).unwrap(),
        |t| crate::record!(f, t.join("file"), "", hash_bytes(b"hello")) => false;
        "file_record_but_target_is_a_symlink"
    )]
    #[test_case(
        |t| fs::create_dir(t.join("dir")).unwrap(),
        |t| crate::record!(f, t.join("dir"), "", hash_bytes(b"hello")) => false;
        "file_record_but_target_is_a_directory"
    )]
    #[test_case(
        |t| fs::write(t.join("file"), "").unwrap(),
        |t| crate::record!(f, t.join("file"), "", hash_bytes(b"")) => true;
        "empty_file_matches_empty_content"
    )]
    #[test_case(
        |t| fs::write(t.join("file"), "hello").unwrap(),
        |t| crate::record!(s, t.join("file"), "", t.join("target")) => false;
        "symlink_record_but_target_is_a_regular_file"
    )]
    #[test_case(
        |t| std::os::unix::fs::symlink(t.join("elsewhere"), t.join("link")).unwrap(),
        |t| crate::record!(s, t.join("link"), "", t.join("elsewhere")) => true;
        "symlink_matches_link_target"
    )]
    #[test_case(
        |t| std::os::unix::fs::symlink(t.join("elsewhere"), t.join("link")).unwrap(),
        |t| crate::record!(s, t.join("link"), "", t.join("other")) => false;
        "symlink_differs_from_link_target"
    )]
    #[test_case(
        |_| {},
        |t| crate::record!(s, t.join("link"), "", t.join("elsewhere")) => false;
        "symlink_record_but_target_missing"
    )]
    fn managed_verdict<T: Fn(&Path), F: Fn(&Path) -> StateRecord>(setup: T, record: F) -> bool {
        let tmp = tempdir().unwrap();
        setup(tmp.path());
        is_managed(&record(tmp.path())).unwrap()
    }
}
