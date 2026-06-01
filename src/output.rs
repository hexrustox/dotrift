use crate::eoutput;
use std::path::Path;

use crossterm::style::Stylize;

use crate::config::DeployType;

fn deploy_type_label(t: DeployType) -> &'static str {
    match t {
        DeployType::Symlink => "symlink",
        DeployType::Copy => "file",
        DeployType::Tmpl => "tmpl",
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

pub fn print_dry_create_dir(path: &Path) {
    eoutput!("{} {}", "[CREATE]".green().bold(), path.display());
}

pub fn print_dry_create_file(target: &Path, source: &Path, deploy_type: DeployType) {
    eoutput!(
        "{} {}",
        "[CREATE]".green().bold(),
        portal_str(target, source, deploy_type)
    );
}

pub fn print_dry_remove(path: &Path) {
    eoutput!("{} {}", "[REMOVE]".red().bold(), path.display());
}

pub fn print_managed(target: &Path, source: &Path, deploy_type: DeployType) {
    eoutput!(
        "{} {}",
        "[MANAGED]".green(),
        portal_str(target, source, deploy_type)
    );
}

pub fn print_unmanaged(target: &Path) {
    eoutput!("{} {}", "[UNMANAGED]".yellow(), target.display());
}

pub fn print_warn(msg: impl std::fmt::Display) {
    eoutput!("{} {}", "[WARN]".yellow().bold(), msg);
}

pub fn print_ok(msg: impl std::fmt::Display) {
    eoutput!("{} {}", "[OK]".green().bold(), msg);
}

pub fn print_created_dir(path: &Path) {
    eoutput!("{} {}", "[CREATED]".green().bold(), path.display());
}

pub fn print_created_file(target: &Path, source: &Path, deploy_type: DeployType) {
    eoutput!(
        "{} {}",
        "[CREATED]".green().bold(),
        portal_str(target, source, deploy_type)
    );
}

pub fn print_removed(path: &Path) {
    eoutput!("{} {}", "[REMOVED]".red().bold(), path.display());
}

pub fn print_added(src: &Path, dest: &Path) {
    eoutput!(
        "{} {} -> {}",
        "[ADDED]".green().bold(),
        src.display(),
        dest.display(),
    );
}

pub fn print_summary(msg: impl std::fmt::Display) {
    eoutput!("{} {}", "[SUMMARY]".bold(), msg);
}

#[macro_export]
macro_rules! output {
    ($($arg:tt)*) => {{
        #[cfg(test)]
        {
            let msg = format!($($arg)*);
            $crate::output::test_capture::push(msg);
        }
        #[cfg(not(test))]
        println!($($arg)*);
    }};
}

#[macro_export]
macro_rules! eoutput {
    ($($arg:tt)*) => {{
        #[cfg(test)]
        {
            let msg = format!($($arg)*);
            $crate::output::test_capture::push(msg);
        }
        #[cfg(not(test))]
        eprintln!($($arg)*);
    }};
}

#[cfg(test)]
pub mod test_capture {
    use std::cell::RefCell;

    thread_local! {
        static OUTPUT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    pub fn push(s: String) {
        OUTPUT.with(|v| v.borrow_mut().push(s));
    }

    pub fn take_all() -> String {
        OUTPUT.with(|v| {
            let mut guard = v.borrow_mut();
            let result = guard.join("\n");
            guard.clear();
            result
        })
    }
}
