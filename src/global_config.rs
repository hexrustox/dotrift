//! The per-user global config: `config.toml` under the user's config
//! directory, configuring dotrift across all source directories.
//!
//! Distinct from the control files, the file is parsed as plain TOML and
//! never template-evaluated (ADR-0018). It is strict: unknown sections,
//! unknown properties, and wrong types fail the run rather than silently
//! leaving a setting unapplied.

use std::{fs, io};

use miette::{Result, WrapErr, miette};
use serde::Deserialize;

/// The per-user global config: the configured pager and `apply` behavior.
#[derive(Debug, Default)]
pub struct GlobalConfig {
    pager: Option<PagerCommand>,
    replace_identical: bool,
}

impl GlobalConfig {
    /// Reads the global config, contributing the defaults when the file is
    /// missing. Discovery and validation errors fail the caller.
    pub fn load() -> Result<Self> {
        let path = crate::paths::global_config_path()?;
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => {
                return Err(miette!(error).wrap_err(format!("cannot read `{}`", path.display())));
            }
        };
        let file = toml::from_slice::<FileConfig>(&bytes)
            .map_err(|error| miette!(error))
            .wrap_err_with(|| format!("cannot parse `{}`", path.display()))?;
        Ok(Self::from_file(file))
    }

    /// The configured pager, `None` when unset or empty by `command`.
    pub fn pager(&self) -> Option<&PagerCommand> {
        self.pager.as_ref()
    }

    /// Whether `apply` replaces identical obstructions without prompting.
    pub fn replace_identical(&self) -> bool {
        self.replace_identical
    }

    fn from_file(file: FileConfig) -> Self {
        let pager = file.pager.and_then(|pager| {
            (!pager.command.trim().is_empty()).then_some(PagerCommand {
                command: pager.command,
                args: pager.args,
            })
        });
        Self {
            pager,
            replace_identical: file.apply.replace_identical,
        }
    }
}

/// A pager program from the global config, used verbatim with literal
/// arguments.
#[derive(Debug)]
pub struct PagerCommand {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    #[serde(default)]
    pager: Option<PagerSection>,
    #[serde(default)]
    apply: ApplySection,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PagerSection {
    command: String,
    #[serde(default)]
    args: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplySection {
    #[serde(default, rename = "replace-identical")]
    replace_identical: bool,
}
