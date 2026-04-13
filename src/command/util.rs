use std::fs::File;
use std::hash::Hasher;
use std::io::{BufReader, Read};
use std::path::Path;

use color_eyre::eyre::{Context, Result};
use twox_hash::XxHash64;

const BUFFER_SIZE: usize = 8192;

pub fn hash_file(path: &Path) -> Result<u64> {
    let file =
        File::open(path).wrap_err_with(|| format!("Failed to open `{}`.", path.display()))?;
    let mut reader = BufReader::with_capacity(BUFFER_SIZE, file);
    let mut hasher = XxHash64::with_seed(1);
    let mut buffer = [0u8; BUFFER_SIZE];

    loop {
        let bytes_read = reader
            .read(&mut buffer)
            .wrap_err_with(|| format!("Failed to read from `{}`.", path.display()))?;
        if bytes_read == 0 {
            break;
        }
        hasher.write(&buffer[..bytes_read]);
    }

    Ok(hasher.finish())
}
