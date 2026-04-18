use std::{borrow::Cow, fs, path::PathBuf};

use color_eyre::eyre::{Context, eyre};
use glob::Pattern;
use normalize_path::NormalizePath;

use crate::{
    cli::{AddFlags, OpenEditor},
    command::util::{is_actual_dir, is_glob},
    config::Config,
    error::{GlobError, IoError},
};

pub fn run(
    source_dir: PathBuf,
    file: PathBuf,
    destination: PathBuf,
    flags: AddFlags,
) -> color_eyre::Result<()> {
    let destination = if destination.is_absolute() {
        destination
    } else {
        source_dir.join(destination).normalize()
    };
    let Ok(dest_rel) = destination.strip_prefix(&source_dir) else {
        return Err(eyre!("Destination must be inside source directory"));
    };
    if !matches!(destination.try_exists(), Ok(false)) && !flags.force {
        return Err(eyre!("`{}` already exists.", destination.display()));
    }

    if let Some(parent) = destination.parent() {
        if flags.force {
            let mut current = Some(parent);
            while let Some(path) = current {
                if !matches!(path.try_exists(), Ok(false)) && !is_actual_dir(path) {
                    fs::remove_file(path).remove_file_error(path)?;
                    break;
                }
                current = path.parent();
            }
        }
        fs::create_dir_all(parent).create_dir_error(parent)?;
    }
    if flags.copy {
        fs::copy(&file, &destination).copy_file_error(&file, &destination)?;
    } else {
        fs::rename(&file, &destination).wrap_err({
            format!(
                "Failed to move `{}` to `{}`.",
                file.display(),
                destination.display()
            )
        })?;
    }

    let open_editor = if let Some(open_editor) = flags.editor {
        matches!(open_editor, OpenEditor::Always)
    } else {
        let config = Config::read(&source_dir)?;
        let mut open_editor = true;
        let dest_rel = dest_rel
            .as_os_str()
            .to_str()
            .map(Cow::Borrowed)
            .unwrap_or_else(|| dest_rel.to_string_lossy());
        for (pattern, _) in config.portal {
            if is_glob(&pattern) && {
                let glob = Pattern::new(&pattern).glob_error()?;
                glob.matches(&dest_rel)
            } || dest_rel == pattern
                || dest_rel.starts_with(&pattern)
            {
                open_editor = false;
                break;
            }
        }
        open_editor
    };

    #[cfg(test)]
    {
        use crate::command::add::tests::OPEN_EDITOR;

        OPEN_EDITOR.set(open_editor);
    }

    if open_editor {
        // TODO
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use crate::command::util::tests::setup_test;

    use super::*;
    use test_case::test_case;

    thread_local! {
        pub static OPEN_EDITOR:RefCell<bool> = const { RefCell::new(false) };
    }

    #[test_case("", "file" => true; "empty")]
    #[test_case(r#""" = """#, "file" => false; "match_root")]
    #[test_case(r#""**" = """#, "file" => false; "match_glob")]
    #[test_case(r#""dir" = """#, "dir/file" => false; "match_parent")]
    #[test_case(r#""dir/file" = """#, "dir/file" => false; "exact_match")]
    #[test_case(r#""*.txt" = """#, "file" => true; "mismatch_glob")]
    #[test_case(r#""subdir" = """#, "file" => true; "mismatch_literal")]
    fn test_open_editor(portal: &str, dest: impl Into<PathBuf>) -> bool {
        let (temp_dir, source_dir, _) = setup_test(portal, "", "", false);

        let path = temp_dir.path().join("file");
        fs::write(&path, "").unwrap();
        run(
            source_dir.clone(),
            path,
            source_dir.join(dest.into()),
            AddFlags {
                copy: false,
                force: false,
                editor: None,
            },
        )
        .unwrap();

        OPEN_EDITOR.with_borrow(|b| *b)
    }
}
