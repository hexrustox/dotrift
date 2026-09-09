//! The management state: the state database, its records and lock, and the
//! fingerprint checks that compare a target path against its record.

mod database;
mod fingerprint;

pub(crate) use database::StateLock;
pub use database::{Kind, StateDatabase, StateRecord, load_active_profiles};
pub use fingerprint::{Fingerprint, TemplateHash, hash_bytes};
pub(crate) use fingerprint::{HashWriter, is_identical, is_managed};
