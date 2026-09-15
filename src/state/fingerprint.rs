//! The fingerprint of deployed bytes: the single module that produces,
//! records, and compares content hashes.
//!
//! A [`Fingerprint`] is the hash of deployed bytes (what lands on disk for a
//! copy or template deploy); a [`TemplateHash`] is the digest of a template's
//! source bytes (what keys the template render registry). The two were both
//! plain strings, so nothing stopped a caller from comparing one against the
//! other; the types now carry the distinction from `spec/CONTEXT.md`.
//!
//! Both read-side comparisons live here so the filesystem probing stays in one
//! place: [`is_managed`] (the managed check, record vs disk) inspects without
//! following symlinks, while [`is_identical`] (the identical-obstruction
//! check, disk vs desired bytes) resolves symlinks. Any read or render failure
//! during either check means "not established", never an error to the caller.

use std::{
    collections::HashMap,
    fs::{self, File},
    hash::Hasher,
    io::{BufReader, Read, Write},
    path::Path,
};

use miette::{Result, WrapErr, miette};
use templater::value::Value;
use twox_hash::XxHash64;

use crate::{
    config::{DeployType, DeploymentEntry},
    render::RenderRegistry,
    state::{Kind, StateRecord},
};

/// The hash of deployed bytes: the recorded last-applied state of a target
/// path dotrift created for a copy or template deploy.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Fingerprint(String);

impl Fingerprint {
    /// Computes the fingerprint of `bytes`.
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(xxhash_hex(bytes))
    }

    /// Computes the fingerprint of the file at `path`, following symlinks.
    pub(crate) fn of_file(path: &Path) -> Result<Self> {
        Ok(Self(xxhash_file_hex(path)?))
    }

    /// Borrows the hex digest, for comparing against a stored state record.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Fingerprint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<Fingerprint> for String {
    fn from(fingerprint: Fingerprint) -> Self {
        fingerprint.0
    }
}

/// The digest of a template's source bytes, computed before render and
/// following symlinks. Keys the template render registry and the run's
/// in-memory memo of rendered-output digests.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TemplateHash(String);

impl TemplateHash {
    /// Computes the template hash of `bytes`.
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(xxhash_hex(bytes))
    }

    /// Computes the template hash of the file at `path`, following symlinks.
    pub(crate) fn of_file(path: &Path) -> Result<Self> {
        Ok(Self(xxhash_file_hex(path)?))
    }

    /// Borrows the hex digest, for naming a registry entry.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TemplateHash {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// An [`Write`] adapter that fingerprints every accepted byte while
/// forwarding it, producing the same digest as [`hash_bytes`] for the same
/// byte stream.
pub(crate) struct HashWriter<W> {
    inner: W,
    hasher: XxHash64,
}

impl<W> HashWriter<W> {
    pub(crate) fn new(inner: W) -> Self {
        Self {
            inner,
            hasher: XxHash64::with_seed(SEED),
        }
    }

    /// Consumes the adapter, returning the fingerprint of everything written.
    pub(crate) fn into_digest(self) -> Fingerprint {
        Fingerprint(format!("{:016x}", self.hasher.finish()))
    }
}

impl<W: Write> Write for HashWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.hasher.write(&buf[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Computes the fingerprint of `bytes`.
pub fn hash_bytes(bytes: &[u8]) -> Fingerprint {
    Fingerprint::of_bytes(bytes)
}

/// Whether the target path is still a managed path: dotrift created it and
/// its current kind and fingerprint still match its state record.
///
/// The target is inspected without following symlinks, so a symlink swapped
/// for a file (or the reverse) fails the kind check. Anything unreadable
/// fails the check and is not a managed path.
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
            match Fingerprint::of_file(&record.target_path) {
                Ok(fingerprint) => Ok(record.content_hash.as_deref() == Some(fingerprint.as_str())),
                Err(_) => Ok(false),
            }
        }
    }
}

/// Whether the entry's own target path is an *identical obstruction*: for a
/// symlink deploy, a symlink whose link target equals the source path; for a
/// file deploy, a path resolving to a regular file whose content fingerprint
/// equals the fingerprint of the bytes that would be deployed. Any failure to
/// read a path or obtain the rendered bytes means the check cannot establish
/// identity and the obstruction is treated as not identical.
pub(crate) fn is_identical(
    entry: &DeploymentEntry,
    context: &HashMap<String, Value>,
    registry: &mut RenderRegistry,
) -> bool {
    match entry.deploy_type {
        DeployType::Symlink => {
            fs::read_link(&entry.target_path).is_ok_and(|link| link == entry.source_path)
        }
        DeployType::Copy => file_matches(&entry.target_path, &entry.source_path),
        DeployType::Template => {
            let Ok(Some(rendered)) = registry.ensure_rendered(&entry.source_path, context) else {
                return false;
            };
            file_matches_digest(&entry.target_path, &rendered.digest)
        }
    }
}

const SEED: u64 = 0;
const CHUNK_SIZE: usize = 64 * 1024;

fn xxhash_hex(bytes: &[u8]) -> String {
    let mut hasher = XxHash64::with_seed(SEED);
    hasher.write(bytes);
    format!("{:016x}", hasher.finish())
}

fn xxhash_file_hex(path: &Path) -> Result<String> {
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

/// Whether `path` resolves, following symlinks, to a regular file holding the
/// same bytes as `source`. File mode is not part of the comparison.
fn file_matches(path: &Path, source: &Path) -> bool {
    Fingerprint::of_file(source)
        .is_ok_and(|source_fingerprint| file_matches_digest(path, &source_fingerprint))
}

/// Whether `path` resolves, following symlinks, to a regular file whose
/// content fingerprint equals `fingerprint`.
fn file_matches_digest(path: &Path, fingerprint: &Fingerprint) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
        && Fingerprint::of_file(path).is_ok_and(|hash| hash == *fingerprint)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use test_case::test_case;

    use super::*;

    #[test_case(b"content1".to_vec() ; "content1_digests_same_as_bytes")]
    #[test_case(Vec::new() ; "empty_digests_same_as_bytes")]
    #[test_case(vec![b'x'; CHUNK_SIZE * 2 + 1] ; "larger_than_chunk_digests_same_as_bytes")]
    fn hash_file_digests_same_as_its_bytes(content: Vec<u8>) {
        let dir = tempdir().expect("cannot create temp dir");
        let path = dir.path().join("sample");
        fs::write(&path, &content).expect("cannot write sample file");
        assert_eq!(
            Fingerprint::of_file(&path).expect("cannot hash sample file"),
            hash_bytes(&content)
        );
    }

    #[test]
    fn hash_file_errs_on_missing_path() {
        let dir = tempdir().expect("cannot create temp dir");
        assert!(Fingerprint::of_file(&dir.path().join("file1")).is_err());
    }

    #[test_case(&[] ; "hash_writer_digests_empty_stream_like_hash_bytes")]
    #[test_case(&[b"content1".as_slice()] ; "hash_writer_digests_one_write_like_hash_bytes")]
    #[test_case(
        &[b"foo".as_slice(), b"bar".as_slice(), b"baz".as_slice()] ;
        "hash_writer_digests_chunked_writes_like_hash_bytes"
    )]
    fn hash_writer_digests_stream_like_hash_bytes(chunks: &[&[u8]]) {
        let mut writer = HashWriter::new(Vec::<u8>::new());
        for chunk in chunks {
            writer.write_all(chunk).expect("cannot write");
        }
        assert_eq!(writer.into_digest(), hash_bytes(&chunks.concat()));
    }

    #[test_case(
        |t| fs::write(t.join("file1"), "content1").unwrap(),
        |t| crate::record!(f, t.join("file1"), hash_bytes(b"content1")) => true;
        "file_content_matches_record_is_managed"
    )]
    #[test_case(
        |t| fs::write(t.join("file1"), "content1").unwrap(),
        |t| crate::record!(f, t.join("file1"), hash_bytes(b"content2")) => false;
        "file_content_diverged_from_record_not_managed"
    )]
    #[test_case(
        |_| {},
        |t| crate::record!(f, t.join("file1"), hash_bytes(b"content1")) => false;
        "file_record_target_missing_not_managed"
    )]
    #[test_case(
        |t| std::os::unix::fs::symlink(t.join("target2"), t.join("file1")).unwrap(),
        |t| crate::record!(f, t.join("file1"), hash_bytes(b"content1")) => false;
        "file_record_target_is_symlink_not_managed"
    )]
    #[test_case(
        |t| fs::create_dir(t.join("dir1")).unwrap(),
        |t| crate::record!(f, t.join("dir1"), hash_bytes(b"content1")) => false;
        "file_record_target_is_directory_not_managed"
    )]
    #[test_case(
        |t| fs::write(t.join("file1"), "").unwrap(),
        |t| crate::record!(f, t.join("file1"), hash_bytes(b"")) => true;
        "empty_file_matches_empty_record_is_managed"
    )]
    #[test_case(
        |t| fs::write(t.join("file1"), "content1").unwrap(),
        |t| crate::record!(s, t.join("file1"), t.join("target1")) => false;
        "symlink_record_target_is_regular_file_not_managed"
    )]
    #[test_case(
        |t| std::os::unix::fs::symlink(t.join("target1"), t.join("link1")).unwrap(),
        |t| crate::record!(s, t.join("link1"), t.join("target1")) => true;
        "symlink_to_recorded_source_path_is_managed"
    )]
    #[test_case(
        |t| std::os::unix::fs::symlink(t.join("target1"), t.join("link1")).unwrap(),
        |t| crate::record!(s, t.join("link1"), t.join("target2")) => false;
        "symlink_to_other_source_path_not_managed"
    )]
    #[test_case(
        |_| {},
        |t| crate::record!(s, t.join("link1"), t.join("target1")) => false;
        "symlink_record_target_missing_not_managed"
    )]
    fn is_managed_when_target_matches_record(
        setup: impl Fn(&std::path::Path),
        record: impl Fn(&std::path::Path) -> StateRecord,
    ) -> bool {
        let tmp = tempdir().unwrap();
        setup(tmp.path());
        is_managed(&record(tmp.path())).unwrap()
    }

    fn entry(source: &Path, target: &Path, deploy_type: DeployType) -> DeploymentEntry {
        DeploymentEntry {
            source_path: source.to_path_buf(),
            target_path: target.to_path_buf(),
            deploy_type,
            mode: None,
        }
    }

    fn no_context() -> HashMap<String, Value> {
        HashMap::new()
    }

    fn dry_registry(anchor: &Path) -> RenderRegistry {
        RenderRegistry::acquire(&crate::platform::Environment::test_root(anchor), true)
    }

    #[test_case(
        |t| {
            fs::write(t.join("file1"), "content1").unwrap();
            fs::write(t.join("target1"), "content1").unwrap();
        } => true ;
        "copy_target_matches_source_bytes_is_identical"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("file1"), "content1").unwrap();
            fs::write(t.join("target1"), "content2").unwrap();
        } => false ;
        "copy_target_diverged_from_source_not_identical"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("file1"), "content1").unwrap();
            fs::create_dir(t.join("target1")).unwrap();
            fs::write(t.join("target1/file1"), "content1").unwrap();
        } => false ;
        "copy_target_is_directory_not_identical"
    )]
    fn is_identical_when_copy_target_matches_source(setup: impl Fn(&Path)) -> bool {
        let dir = tempdir().unwrap();
        setup(dir.path());
        let entry = entry(
            &dir.path().join("file1"),
            &dir.path().join("target1"),
            DeployType::Copy,
        );
        let mut registry = dry_registry(dir.path());

        is_identical(&entry, &no_context(), &mut registry)
    }

    #[test_case(
        |t| {
            fs::write(t.join("file1"), "content1").unwrap();
            std::os::unix::fs::symlink(t.join("file1"), t.join("target1")).unwrap();
        } => true ;
        "symlink_target_points_at_source_is_identical"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("file2"), "content1").unwrap();
            std::os::unix::fs::symlink(t.join("file2"), t.join("target1")).unwrap();
        } => false ;
        "symlink_target_points_elsewhere_not_identical"
    )]
    fn is_identical_when_symlink_target_matches_source(setup: impl Fn(&Path)) -> bool {
        let dir = tempdir().unwrap();
        setup(dir.path());
        let entry = entry(
            &dir.path().join("file1"),
            &dir.path().join("target1"),
            DeployType::Symlink,
        );
        let mut registry = dry_registry(dir.path());

        is_identical(&entry, &no_context(), &mut registry)
    }

    #[test_case("str\n", false => true ; "template_target_matches_render_is_identical")]
    #[test_case("content1\n", false => false ; "template_target_diverged_from_render_not_identical")]
    #[test_case("str\n", true => false ; "template_render_unavailable_not_identical")]
    fn is_identical_when_template_target_matches_render(target_content: &str, dry: bool) -> bool {
        let source = tempdir().unwrap();
        let target = tempdir().unwrap();
        let registry_root = tempdir().unwrap();
        fs::write(source.path().join("file1"), "{{ str }}\n").unwrap();
        fs::write(target.path().join("target1"), target_content).unwrap();
        let entry = entry(
            &source.path().join("file1"),
            &target.path().join("target1"),
            DeployType::Template,
        );
        let context = HashMap::from([("str".to_string(), Value::Str("str".into()))]);
        let env = crate::platform::Environment::test_root(registry_root.path());
        let mut registry = if dry {
            dry_registry(source.path())
        } else {
            RenderRegistry::acquire(&env, false)
        };

        is_identical(&entry, &context, &mut registry)
    }
}
