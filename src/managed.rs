use std::fs;
use std::hash::Hasher;

use miette::Result;
use twox_hash::XxHash64;

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
