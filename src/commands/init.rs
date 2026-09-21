use std::fs;
use std::io::Write;
use std::path::Path;

use miette::{Result, WrapErr, miette};

use crate::platform::{Environment, prettify_path};
use crate::report::Reporter;

const SCAFFOLD_DOTRIFT_TOML: &str = r#"# Maps source paths to target paths. Rendered as a template before
# parsing: template tags in this file are evaluated, so keep examples
# commented out unless you want them rendered.

# target-directory = "/absolute/path"

# [portal]
# "config/**/*.toml" = ".config"
# "file1" = ".file1"

# [rule]
# ".config/**" = { type = "copy" }
# ".config/secrets/**" = { mode = "600" }
"#;

const SCAFFOLD_DATA_FILE: &str = r#"# Base variables and profiles for template rendering.

# [variable]
# str = "str"
# num = 1

# [profile.profile1]
# str = "profile1"
"#;

const SCAFFOLD_IGNORE_FILE: &str = r#"# Excludes resolved target paths from deployment. Gitignore-style
# patterns: a pattern containing no slash matches a file name at any
# depth; a pattern containing a slash is anchored to the
# target-directory root.

# file1
# dir1/**
"#;

/// Creates the missing control files in the source directory, scaffolding a
/// fresh one when the directory itself does not exist (`spec/commands/init.md`).
pub fn run(source: &Path, _env: &Environment, color: bool) -> Result<()> {
    let report = Reporter::always(color);

    match fs::metadata(source) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(miette!(
                "source directory `{}` is not a directory",
                source.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // A dangling symlink exists as a directory entry but resolves to
            // nothing: never create the directory it points at.
            if fs::symlink_metadata(source).is_ok() {
                return Err(miette!(
                    "source directory `{}` does not resolve to a directory",
                    source.display()
                ));
            }
            fs::create_dir_all(source)
                .map_err(|error| miette!(error))
                .wrap_err_with(|| {
                    format!("cannot create source directory `{}`", source.display())
                })?;
        }
        Err(error) => {
            return Err(miette!(error).wrap_err(format!(
                "cannot access source directory `{}`",
                source.display()
            )));
        }
    }

    if entry_exists(&source.join("dotrift.toml")) {
        return Err(miette!(
            "source directory `{}` is already initialized: `dotrift.toml` exists",
            source.display()
        ));
    }

    let mut created = Vec::new();
    for (name, scaffold) in [
        ("dotrift.toml", SCAFFOLD_DOTRIFT_TOML),
        ("dotrift_data.toml", SCAFFOLD_DATA_FILE),
        (".dotriftignore", SCAFFOLD_IGNORE_FILE),
    ] {
        let path = source.join(name);
        if entry_exists(&path) {
            continue;
        }
        // `create_new` is exclusive: it fails rather than following or
        // writing through an existing entry, so a concurrent creator cannot
        // be clobbered.
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| miette!(error))
            .wrap_err_with(|| format!("cannot create `{}`", path.display()))?
            .write_all(scaffold.as_bytes())
            .map_err(|error| miette!(error))
            .wrap_err_with(|| format!("cannot write `{}`", path.display()))?;
        created.push(path);
    }
    // Errors print nothing to standard output, so the listing is held back
    // until every missing file is written.
    for path in created {
        report.line(format_args!("{}", prettify_path(&path).display()));
    }
    Ok(())
}

/// Whether a directory entry of any kind exists at `path`, without following
/// symlinks (`spec/commands/init.md § Presence`).
fn entry_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}
