use std::fs;

use miette::Result;

use crate::{
    hash,
    state::{Kind, StateRecord},
};

pub(crate) fn is_managed(record: &StateRecord) -> Result<bool> {
    let metadata = match fs::symlink_metadata(&record.target_path) {
        Ok(metadata) => metadata,
        Err(_) => return Ok(false),
    };

    match record.kind {
        Kind::Symlink => {
            if !metadata.file_type().is_symlink() {
                return Ok(false);
            }
            Ok(matches!(
                fs::read_link(&record.target_path),
                Ok(link) if link == record.source_path
            ))
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
    use std::path::Path;

    use tempfile::tempdir;
    use test_case::test_case;

    use crate::hash::hash_bytes;

    use super::*;

    #[test_case(
        |t| fs::write(t.join("file"), "hello").unwrap(),
        |t| crate::record!(f, t.join("file"), hash_bytes(b"hello")) => true;
        "file_content_matches_record_is_managed"
    )]
    #[test_case(
        |t| fs::write(t.join("file"), "hello").unwrap(),
        |t| crate::record!(f, t.join("file"), hash_bytes(b"world")) => false;
        "file_content_diverged_from_record_not_managed"
    )]
    #[test_case(
        |_| {},
        |t| crate::record!(f, t.join("file"), hash_bytes(b"hello")) => false;
        "file_record_target_missing_not_managed"
    )]
    #[test_case(
        |t| std::os::unix::fs::symlink(t.join("elsewhere"), t.join("file")).unwrap(),
        |t| crate::record!(f, t.join("file"), hash_bytes(b"hello")) => false;
        "file_record_target_is_symlink_not_managed"
    )]
    #[test_case(
        |t| fs::create_dir(t.join("dir")).unwrap(),
        |t| crate::record!(f, t.join("dir"), hash_bytes(b"hello")) => false;
        "file_record_target_is_directory_not_managed"
    )]
    #[test_case(
        |t| fs::write(t.join("file"), "").unwrap(),
        |t| crate::record!(f, t.join("file"), hash_bytes(b"")) => true;
        "empty_file_matches_empty_record_is_managed"
    )]
    #[test_case(
        |t| fs::write(t.join("file"), "hello").unwrap(),
        |t| crate::record!(s, t.join("file"), t.join("target")) => false;
        "symlink_record_target_is_regular_file_not_managed"
    )]
    #[test_case(
        |t| std::os::unix::fs::symlink(t.join("elsewhere"), t.join("link")).unwrap(),
        |t| crate::record!(s, t.join("link"), t.join("elsewhere")) => true;
        "symlink_to_recorded_source_path_is_managed"
    )]
    #[test_case(
        |t| std::os::unix::fs::symlink(t.join("elsewhere"), t.join("link")).unwrap(),
        |t| crate::record!(s, t.join("link"), t.join("other")) => false;
        "symlink_to_other_source_path_not_managed"
    )]
    #[test_case(
        |_| {},
        |t| crate::record!(s, t.join("link"), t.join("elsewhere")) => false;
        "symlink_record_target_missing_not_managed"
    )]
    fn is_managed_when_target_matches_record(
        setup: impl Fn(&Path),
        record: impl Fn(&Path) -> StateRecord,
    ) -> bool {
        let tmp = tempdir().unwrap();
        setup(tmp.path());
        is_managed(&record(tmp.path())).unwrap()
    }
}
