pub mod cli;
pub mod commands;
pub mod config;
pub mod deploy;
pub mod platform;
mod render;
pub mod report;
pub mod state;

pub(crate) fn internal_error(message: impl Into<String>) -> miette::Report {
    miette::MietteDiagnostic::new(message.into())
        .with_help("this is likely an internal error")
        .into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    Success = 0,
    Cancelled = 1,
    Skipped = 2,
}
