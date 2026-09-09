//! Content hashing primitives, re-exported from the fingerprint module.
//!
//! The fingerprint module owns digest production and comparison; this module
//! stays as the crate's public interface for hashing because integration
//! tests and the state record's stored digest use it.

pub use crate::fingerprint::{Fingerprint, TemplateHash, hash_bytes};
