//! Dotrift-owned file locations: the directories and files dotrift reads and
//! writes outside the source tree, resolved from the XDG base directories.
//!
//! The single **adapter** behind the location **seam**: production code
//! resolves it once per run via [`Environment::resolve`], tests build a
//! hermetic one via [`Environment::test_root`]. No caller touches the
//! environment directly, so XDG fallback knowledge has **locality** here and
//! one fake root serves every test.

use std::path::{Path, PathBuf};

use miette::{Result, WrapErr, miette};

/// Where dotrift reads and writes outside the source tree.
///
/// Constructed from the process environment in production and from an
/// explicit root in tests. Each accessor resolves lazily, preserving the
/// previous per-resolver failure semantics: only the locations a command
/// actually needs can fail it.
#[derive(Debug, Clone, Default)]
pub struct Environment {
    state_dir: Option<PathBuf>,
    registry_dir: Option<PathBuf>,
    global_config_path: Option<PathBuf>,
    default_source_dir: Option<PathBuf>,
    default_target_dir: Option<PathBuf>,
}

impl Environment {
    /// Resolves locations from the process environment on each access.
    pub fn resolve() -> Self {
        Self::default()
    }

    /// Builds a hermetic environment rooted at `root`:
    /// `<root>/state`, `<root>/render-registry/registry`, and
    /// `<root>/config-home/dotrift/config.toml`.
    ///
    /// The default source and target directories are deliberately *not*
    /// overridden: they always resolve from the process environment so tests
    /// exercise the real fallback chain (e.g. the home fallback).
    pub fn test_root(root: &Path) -> Self {
        Self {
            state_dir: Some(root.join("state")),
            registry_dir: Some(root.join("render-registry").join("registry")),
            global_config_path: Some(root.join("config-home").join("dotrift").join("config.toml")),
            default_source_dir: None,
            default_target_dir: None,
        }
    }

    /// Overrides the template render registry directory, for tests that need
    /// registry infrastructure to fail (e.g. pointing at a regular file).
    #[cfg(test)]
    pub(crate) fn with_registry_dir(mut self, dir: PathBuf) -> Self {
        self.registry_dir = Some(dir);
        self
    }

    /// The state directory holding the database and lock:
    /// `$XDG_STATE_HOME/dotrift`, falling back to `$XDG_DATA_HOME/dotrift`
    /// (`spec/core.md § State database`).
    pub(crate) fn state_dir(&self) -> Result<PathBuf> {
        if let Some(root) = self.state_dir.clone() {
            return Ok(root);
        }
        dirs::state_dir()
            .or_else(dirs::data_dir)
            .map(|state_home| state_home.join("dotrift"))
            .ok_or_else(|| {
                miette!(
                    help = "set `XDG_STATE_HOME` or `XDG_DATA_HOME`",
                    "`XDG_STATE_HOME` and `XDG_DATA_HOME` are unset"
                )
            })
            .wrap_err("cannot resolve state location")
    }

    /// The per-run template render registry directory:
    /// `<temporary directory>/dotrift-render/registry` (ADR-0017).
    pub(crate) fn registry_dir(&self) -> PathBuf {
        if let Some(dir) = self.registry_dir.clone() {
            return dir;
        }
        std::env::temp_dir().join("dotrift-render").join("registry")
    }

    /// The global config file: `$XDG_CONFIG_HOME/dotrift/config.toml`, falling
    /// back to `$HOME/.config/dotrift/config.toml` (`spec/global-config.md`).
    pub(crate) fn global_config_path(&self) -> Result<PathBuf> {
        if let Some(path) = self.global_config_path.clone() {
            return Ok(path);
        }
        dirs::config_dir()
            .map(|config_home| config_home.join("dotrift").join("config.toml"))
            .ok_or_else(|| {
                miette!(
                    help = "set `XDG_CONFIG_HOME` or `HOME`",
                    "`HOME` is unset or empty"
                )
            })
            .wrap_err("cannot resolve the global config location")
    }

    /// The default source directory: `$XDG_DATA_HOME/dotfiles`, falling back to
    /// `$HOME/.local/share/dotfiles` (`spec/commands/global.md`).
    pub(crate) fn default_source_dir(&self) -> Result<PathBuf> {
        if let Some(dir) = self.default_source_dir.clone() {
            return Ok(dir);
        }
        dirs::data_dir()
            .map(|data_home| data_home.join("dotfiles"))
            .ok_or_else(|| {
                miette!(
                    help = "set `XDG_DATA_HOME` or `HOME`",
                    "both `XDG_DATA_HOME` and `HOME` are unset"
                )
            })
            .wrap_err("cannot resolve source directory")
    }

    /// The terminal fallback of the target resolution chain: the user's home
    /// directory, used when neither the CLI nor `dotrift.toml` names a target.
    pub(crate) fn default_target_dir(&self) -> Result<PathBuf> {
        if let Some(dir) = self.default_target_dir.clone() {
            return Ok(dir);
        }
        dirs::home_dir()
            .ok_or_else(|| miette!(help = "set `HOME`", "`HOME` is unset or empty"))
            .wrap_err("cannot resolve the home directory")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard};

    use super::*;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct VarGuard {
        previous: Vec<(&'static str, Option<String>)>,
        _guard: MutexGuard<'static, ()>,
    }

    impl VarGuard {
        fn set<'a>(vars: impl IntoIterator<Item = (&'static str, Option<&'a str>)>) -> Self {
            let _guard = ENV_LOCK.lock().expect("env lock poisoned");
            let previous = vars
                .into_iter()
                .map(|(name, value)| {
                    let previous = std::env::var(name).ok();
                    unsafe {
                        match value {
                            Some(value) => std::env::set_var(name, value),
                            None => std::env::remove_var(name),
                        }
                    }
                    (name, previous)
                })
                .collect();
            Self { previous, _guard }
        }
    }

    impl Drop for VarGuard {
        fn drop(&mut self) {
            for (name, previous) in &self.previous {
                unsafe {
                    match previous {
                        Some(previous) => std::env::set_var(name, previous),
                        None => std::env::remove_var(name),
                    }
                }
            }
        }
    }

    fn temp_dir_with_name(name: &'static str) -> PathBuf {
        tempfile::tempdir()
            .expect("cannot create temp dir")
            .keep()
            .join(name)
    }

    fn temporary_home() -> String {
        tempfile::tempdir()
            .expect("cannot create temp dir")
            .keep()
            .display()
            .to_string()
    }

    #[test]
    fn test_root_overrides_the_private_locations() {
        let root = tempfile::tempdir().expect("cannot create temp dir");
        let env = Environment::test_root(root.path());
        assert_eq!(env.state_dir().unwrap(), root.path().join("state"));
        assert_eq!(
            env.registry_dir(),
            root.path().join("render-registry").join("registry")
        );
        assert_eq!(
            env.global_config_path().unwrap(),
            root.path()
                .join("config-home")
                .join("dotrift")
                .join("config.toml")
        );
    }

    #[test]
    fn with_registry_dir_overrides_the_registry_location() {
        let env = Environment::default().with_registry_dir(PathBuf::from("file1"));
        assert_eq!(env.registry_dir(), PathBuf::from("file1"));
    }

    #[test]
    fn registry_dir_defaults_to_the_temporary_directory() {
        let _guard = VarGuard::set([]);
        let expected = std::env::temp_dir().join("dotrift-render").join("registry");
        assert_eq!(Environment::default().registry_dir(), expected);
    }

    #[test]
    fn state_dir_prefers_the_state_home() {
        let state = temp_dir_with_name("1");
        let _guard = VarGuard::set([("XDG_STATE_HOME", Some(state.to_str().unwrap()))]);
        assert_eq!(
            Environment::default().state_dir().unwrap(),
            state.join("dotrift")
        );
    }

    #[test]
    fn state_dir_falls_back_to_the_home_state_directory() {
        let home = temporary_home();
        let _guard = VarGuard::set([("XDG_STATE_HOME", None), ("HOME", Some(home.as_str()))]);
        assert_eq!(
            Environment::default().state_dir().unwrap(),
            PathBuf::from(&home)
                .join(".local")
                .join("state")
                .join("dotrift")
        );
    }

    #[test]
    fn global_config_path_prefers_the_config_home() {
        let config = temp_dir_with_name("1");
        let _guard = VarGuard::set([
            ("XDG_CONFIG_HOME", Some(config.to_str().unwrap())),
            ("HOME", None),
        ]);
        assert_eq!(
            Environment::default().global_config_path().unwrap(),
            config.join("dotrift").join("config.toml")
        );
    }

    #[test]
    fn default_source_dir_uses_the_data_home() {
        let data = temp_dir_with_name("1");
        let _guard = VarGuard::set([("XDG_DATA_HOME", Some(data.to_str().unwrap()))]);
        assert_eq!(
            Environment::default().default_source_dir().unwrap(),
            data.join("dotfiles")
        );
    }

    #[test]
    fn default_target_dir_is_the_home_directory() {
        let home = tempfile::tempdir().expect("cannot create temp dir");
        let _guard = VarGuard::set([("HOME", Some(home.path().to_str().unwrap()))]);
        assert_eq!(
            Environment::default().default_target_dir().unwrap(),
            home.path()
        );
    }
}
