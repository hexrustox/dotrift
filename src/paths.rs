//! Dotrift-owned file locations: the directories and files dotrift reads and
//! writes outside the source tree, resolved from the XDG base directories.

use std::path::{Path, PathBuf};

use miette::{Result, WrapErr, miette};

/// The state directory holding the database and lock:
/// `$XDG_STATE_HOME/dotrift`, falling back to `$XDG_DATA_HOME/dotrift`
/// (`spec/core.md § State database`).
pub(crate) fn state_dir() -> Result<PathBuf> {
    #[cfg(any(test, feature = "testing"))]
    if let Some(root) = test_hooks::TEST_STATE_DIR.with(|cell| cell.borrow().clone()) {
        return Ok(root);
    }
    let state_home = dirs::state_dir()
        .or_else(dirs::data_dir)
        .map(|state_home| state_home.join("dotrift"))
        .ok_or_else(|| miette!("XDG_STATE_HOME and XDG_DATA_HOME are unset"))
        .wrap_err("cannot resolve state location")?;
    Ok(state_home)
}

/// The per-run template render registry directory:
/// `<temporary directory>/dotrift-render/registry` (ADR-0017).
pub(crate) fn registry_dir() -> PathBuf {
    #[cfg(any(test, feature = "testing"))]
    if let Some(dir) = test_hooks::TEST_REGISTRY_DIR.with(|cell| cell.borrow().clone()) {
        return dir;
    }
    std::env::temp_dir().join("dotrift-render").join("registry")
}

/// The global config file: `$XDG_CONFIG_HOME/dotrift/config.toml`, falling
/// back to `$HOME/.config/dotrift/config.toml` (`spec/global-config.md`).
pub(crate) fn global_config_path() -> Result<PathBuf> {
    #[cfg(any(test, feature = "testing"))]
    if let Some(path) = test_hooks::TEST_GLOBAL_CONFIG_PATH.with(|cell| cell.borrow().clone()) {
        return Ok(path);
    }
    dirs::config_dir()
        .map(|config_home| config_home.join("dotrift").join("config.toml"))
        .ok_or_else(|| miette!("`HOME` is unset or empty"))
        .wrap_err("cannot resolve the global config location")
}

/// The default source directory: `$XDG_DATA_HOME/dotfiles`, falling back to
/// `$HOME/.local/share/dotfiles` (`spec/commands/global.md`).
pub(crate) fn default_source_dir() -> Result<PathBuf> {
    dirs::data_dir()
        .map(|data_home| data_home.join("dotfiles"))
        .ok_or_else(|| miette!("both XDG_DATA_HOME and HOME are unset"))
        .wrap_err("cannot resolve source directory")
}

/// The terminal fallback of the target resolution chain: the user's home
/// directory, used when neither the CLI nor `dotrift.toml` names a target.
pub(crate) fn default_target_dir() -> Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| miette!("`HOME` is unset or empty"))
}

/// The state database file under the state directory `root`
/// (`spec/core.md § State database`).
pub(crate) fn state_database_path(root: &Path) -> PathBuf {
    root.join("state.sqlite")
}

/// The state lock file under the state directory `root`
/// (`spec/core.md § State lock`).
pub(crate) fn state_lock_path(root: &Path) -> PathBuf {
    root.join("state.lock")
}

/// Per-resolver test hooks: a set hook replaces the environment chain
/// wholesale, keeping tests off the real user configuration. Each hook holds
/// the resolver's full return value, returned verbatim.
#[cfg(any(test, feature = "testing"))]
pub mod test_hooks {
    use std::{cell::RefCell, path::PathBuf};

    thread_local! {
        /// Overrides [`crate::paths::state_dir`].
        pub static TEST_STATE_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
        /// Overrides [`crate::paths::registry_dir`].
        pub static TEST_REGISTRY_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
        /// Overrides [`crate::paths::global_config_path`].
        pub static TEST_GLOBAL_CONFIG_PATH: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }
}
