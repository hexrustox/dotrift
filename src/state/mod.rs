//! The management state: the state database, its records and lock, and the fingerprint checks that compare a target path against a state record or the desired deployment.

mod database;
mod fingerprint;

pub use database::{Kind, StateDatabase, StateRecord};
pub(crate) use database::{StateLock, load_active_profiles};
pub use fingerprint::{Fingerprint, TemplateHash, hash_bytes};
pub(crate) use fingerprint::{HashWriter, is_identical, is_managed};
