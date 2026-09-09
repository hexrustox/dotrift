//! The apply-time engine: deciding what to do with each desired entry
//! (`reconcile`), performing it (`deployer`), and asking the user about
//! obstructions (`obstruction`).

mod deployer;
pub mod obstruction;
mod reconcile;

pub(crate) use deployer::{DeployOutcome, Deployer, ReplaceLatch, cleanup, describe_decision};
pub use obstruction::{ObstructionChoice, Prompter};
pub(crate) use reconcile::decide;
