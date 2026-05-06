use std::path::Path;

use crossterm::style::Stylize;

use crate::config::DeployType;

fn deploy_type_label(t: DeployType) -> &'static str {
    match t {
        DeployType::Symlink => "symlink",
        DeployType::Copy => "file",
    }
}

pub fn portal_str(target: &Path, source: &Path, deploy_type: DeployType) -> String {
    format!(
        "{} -> {} ({})",
        target.display(),
        source.display(),
        deploy_type_label(deploy_type),
    )
}

pub fn print_created_dir(path: &Path) {
    eprintln!("{} {}", "[CREATE]".green().bold(), path.to_string_lossy());
}

pub fn print_created_file(target: &Path, source: &Path, deploy_type: DeployType) {
    eprintln!(
        "{} {}",
        "[CREATE]".green().bold(),
        portal_str(target, source, deploy_type)
    );
}

pub fn print_removed(path: &Path) {
    eprintln!("{} {}", "[REMOVE]".red().bold(), path.to_string_lossy());
}

pub fn print_managed(target: &Path, source: &Path, deploy_type: DeployType) {
    eprintln!(
        "{} {}",
        "[MANAGED]".green(),
        portal_str(target, source, deploy_type)
    );
}

pub fn print_unmanaged(target: &Path) {
    eprintln!("{} {}", "[UNMANAGED]".yellow(), target.to_string_lossy());
}

pub fn print_warn(msg: impl std::fmt::Display) {
    eprintln!("{} {}", "[WARN]".yellow().bold(), msg);
}

pub fn print_ok(msg: impl std::fmt::Display) {
    eprintln!("{} {}", "[OK]".green().bold(), msg);
}

pub fn print_summary(msg: impl std::fmt::Display) {
    eprintln!("{} {}", "[SUMMARY]".bold(), msg);
}
