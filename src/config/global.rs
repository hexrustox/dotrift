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

use crate::platform::Environment;

/// The per-user global config: the configured pager, the configured diff
/// command, and `apply` behavior.
#[derive(Debug, Default)]
pub struct GlobalConfig {
    pager: Option<PagerCommand>,
    diff: Option<DiffCommand>,
    replace_identical: bool,
}

impl GlobalConfig {
    /// Reads the global config, contributing the defaults when the file is
    /// missing. Discovery and validation errors fail the caller.
    pub fn load(env: &Environment) -> Result<Self> {
        let path = env.global_config_path()?;
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
        Self::from_file(file)
    }

    /// The configured pager, `None` when unset or empty by `command`.
    pub fn pager(&self) -> Option<&PagerCommand> {
        self.pager.as_ref()
    }

    /// The configured diff command, `None` when unset or empty by `command`.
    pub fn diff(&self) -> Option<&DiffCommand> {
        self.diff.as_ref()
    }

    /// Whether `apply` replaces identical obstructions without prompting.
    pub fn replace_identical(&self) -> bool {
        self.replace_identical
    }

    fn from_file(file: FileConfig) -> Result<Self> {
        let pager = file
            .pager
            .filter(|pager| !pager.command.trim().is_empty())
            .map(PagerCommand::from_section);
        let diff = match file.diff {
            None => None,
            Some(section) if section.command.trim().is_empty() => None,
            Some(section) => {
                validate_placeholders(&section.args)?;
                Some(DiffCommand {
                    command: section.command,
                    args: section.args,
                })
            }
        };
        Ok(Self {
            pager,
            diff,
            replace_identical: file.apply.replace_identical,
        })
    }
}

/// A pager program from the global config, used verbatim with literal
/// arguments.
#[derive(Debug, PartialEq, Eq)]
pub struct PagerCommand {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
}

impl PagerCommand {
    fn from_section(section: PagerSection) -> Self {
        Self {
            command: section.command,
            args: section.args,
        }
    }
}

/// A diff program from the global config, used verbatim with literal
/// arguments in which the diff placeholders are substituted at spawn time.
#[derive(Debug)]
pub struct DiffCommand {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
}

/// A `${`…`}` placeholder in a `[diff]` `args` element.
#[derive(Debug)]
enum Placeholder {
    Target,
    Source,
    TargetLabel,
    SourceLabel,
}

impl Placeholder {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "target" => Some(Self::Target),
            "source" => Some(Self::Source),
            "target-label" => Some(Self::TargetLabel),
            "source-label" => Some(Self::SourceLabel),
            _ => None,
        }
    }
}

/// Validates the placeholder rules for `[diff]` args: every `${`…`}` must be
/// a known placeholder, and `${target}` / `${source}` must appear together
/// (`spec/global-config.md § Placeholder validation`).
fn validate_placeholders(args: &[String]) -> Result<()> {
    let mut has_target = false;
    let mut has_source = false;
    for arg in args {
        let mut rest = arg.as_str();
        while let Some(start) = rest.find("${") {
            let Some(end) = rest[start + 2..].find('}') else {
                return Err(miette!(
                    "unterminated placeholder in diff args `{arg}`: expected `${{target}}`, `${{source}}`, `${{target-label}}`, or `${{source-label}}`"
                ));
            };
            let name = &rest[start + 2..start + 2 + end];
            match Placeholder::from_name(name) {
                Some(Placeholder::Target) => has_target = true,
                Some(Placeholder::Source) => has_source = true,
                Some(_) => {}
                None => {
                    return Err(miette!(
                        "unknown placeholder `${{{name}}}` in diff args `{arg}`: expected `${{target}}`, `${{source}}`, `${{target-label}}`, or `${{source-label}}`"
                    ));
                }
            }
            rest = &rest[start + 2 + end + 1..];
        }
    }
    if has_target != has_source {
        return Err(miette!(
            "diff args must reference `${{target}}` and `${{source}}` together, not just one of them"
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    #[serde(default)]
    pager: Option<PagerSection>,
    #[serde(default)]
    diff: Option<DiffSection>,
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DiffSection {
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
