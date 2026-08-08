use std::fs;
use std::hash::Hasher;

use twox_hash::XxHash64;

use crate::state::{Kind, StateError, StateRecord};

pub fn is_managed(record: &StateRecord) -> Result<bool, StateError> {
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
            let bytes = match fs::read(&record.target_path) {
                Ok(bytes) => bytes,
                Err(_) => return Ok(false),
            };
            let mut hasher = XxHash64::with_seed(0);
            hasher.write(&bytes);
            Ok(Some(format!("{:016x}", hasher.finish())) == record.content_hash)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn file_is_managed_when_content_fingerprint_matches() {
        let directory = tempdir().unwrap();
        let target = directory.path().join("target");
        fs::write(&target, b"content").unwrap();
        let mut hasher = XxHash64::with_seed(0);
        hasher.write(b"content");
        let record = StateRecord {
            target_path: target,
            source_path: PathBuf::from("source"),
            kind: Kind::File,
            link_target: None,
            content_hash: Some(format!("{:016x}", hasher.finish())),
        };

        assert!(is_managed(&record).unwrap());
    }

    #[test]
    fn symlink_is_unmanaged_when_link_target_changes() {
        let directory = tempdir().unwrap();
        let target = directory.path().join("target");
        symlink("first", &target).unwrap();
        let record = StateRecord {
            target_path: target,
            source_path: PathBuf::from("source"),
            kind: Kind::Symlink,
            link_target: Some(PathBuf::from("second")),
            content_hash: None,
        };

        assert!(!is_managed(&record).unwrap());
    }
}
