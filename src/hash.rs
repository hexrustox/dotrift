use std::{
    fs::File,
    hash::Hasher,
    io::{BufReader, Read},
    path::Path,
};

use miette::{Result, WrapErr, miette};
use twox_hash::XxHash64;

const SEED: u64 = 0;
const CHUNK_SIZE: usize = 64 * 1024;

pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = XxHash64::with_seed(SEED);
    hasher.write(bytes);
    format!("{:016x}", hasher.finish())
}

pub(crate) fn hash_file(path: &Path) -> Result<String> {
    let file = File::open(path)
        .map_err(|error| miette!(error))
        .wrap_err_with(|| format!("cannot read `{}` for hashing", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = XxHash64::with_seed(SEED);
    let mut buffer = vec![0u8; CHUNK_SIZE];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| miette!(error))
            .wrap_err_with(|| format!("cannot read `{}` for hashing", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.write(&buffer[..read]);
    }
    Ok(format!("{:016x}", hasher.finish()))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn hash_bytes_empty() {
        assert_eq!(hash_bytes(&[]), "ef46db3751d8e999");
    }

    #[test]
    fn hash_bytes_sample() {
        assert_eq!(hash_bytes(b"hello"), "26c7827d889f6da3");
    }

    #[test]
    fn hash_file_matches_hash_bytes() {
        let dir = tempdir().expect("cannot create temp dir");
        let path = dir.path().join("sample");
        fs::write(&path, b"hello").expect("cannot write sample file");
        assert_eq!(
            hash_file(&path).expect("cannot hash sample file"),
            hash_bytes(b"hello")
        );
    }

    #[test]
    fn hash_file_reports_missing_file() {
        let dir = tempdir().expect("cannot create temp dir");
        assert!(hash_file(&dir.path().join("missing")).is_err());
    }
}
