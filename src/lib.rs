pub mod cli;
pub mod commands;
pub mod config;
pub mod deploy;
pub mod platform;
mod render;
pub mod report;
pub mod state;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    Success = 0,
    Cancelled = 1,
    Skipped = 2,
}
