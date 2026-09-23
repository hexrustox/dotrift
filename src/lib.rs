pub mod cli;
pub mod commands;
pub mod config;
pub mod deploy;
pub mod platform;
mod render;
pub mod report;
pub mod state;

/// The process exit code `main` exits with, carried back from a command run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    Success = 0,
    Cancelled = 1,
    Skipped = 2,
}

pub(crate) fn internal_error(message: impl Into<String>) -> miette::Report {
    miette::MietteDiagnostic::new(message.into())
        .with_help("this is likely an internal error")
        .into()
}
