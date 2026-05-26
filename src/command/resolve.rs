use std::collections::HashMap;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{WrapErr, eyre};
use glob::glob_with;
use ignore::gitignore::Gitignore;
use normalize_path::NormalizePath;

use crate::command::util::{
    GLOB_OPTION, PathLiteral, SafeStripPrefix, is_glob, is_ignored, strip_prefix_filter_glob,
    walk_files,
};

pub fn resolve_portal_entries<F>(
    source_dir: &Path,
    target_dir: &Path,
    portals: &HashMap<String, PathBuf>,
    ignore_matcher: &Gitignore,
    skip_missing: bool,
    mut on_entry: F,
) -> color_eyre::Result<()>
where
    F: FnMut(PathBuf, PathBuf, String) -> color_eyre::Result<()>,
{
    for (pattern, target_rel) in portals {
        let pattern_normalized = Path::new(pattern).normalize();
        let target_rel_normalized = target_rel.normalize();
        let pattern_key = pattern_normalized.to_string_lossy().into_owned();

        if is_glob(&pattern_key) {
            let prefix = strip_prefix_filter_glob(&pattern_key);
            let full_pattern = source_dir.join(&pattern_normalized);
            let full_pattern_str = full_pattern.to_string_lossy();

            let paths = match crate::glob_err!(
                glob_with(&full_pattern_str, GLOB_OPTION),
                &full_pattern_str
            ) {
                Ok(p) => p,
                Err(e) => {
                    if skip_missing {
                        continue;
                    }
                    return Err(e);
                }
            };

            for source_path in paths.flatten() {
                if source_path.path_is_dir() {
                    continue;
                }

                let source_rel = source_path.safe_strip_prefix(source_dir);

                let stripped = if prefix.is_empty() {
                    source_rel.to_path_buf()
                } else {
                    source_rel
                        .safe_strip_prefix(Path::new(&prefix))
                        .to_path_buf()
                };

                let target_path = target_dir.join(&target_rel_normalized).join(stripped);

                if is_ignored(ignore_matcher, &target_path) {
                    continue;
                }

                on_entry(source_path, target_path, pattern_key.clone())?;
            }
        } else {
            let source_path = source_dir.join(&pattern_normalized);

            if !source_path.path_exists() {
                if skip_missing {
                    continue;
                }
                return Err(eyre!(
                    "Source path does not exist: `{}`",
                    source_path.display()
                ));
            }

            if source_path.path_is_dir() {
                for entry in walk_files(&source_path) {
                    let file_source = entry.path().to_path_buf();

                    let rel_to_pattern = file_source.safe_strip_prefix(&source_path);

                    let target_path = target_dir.join(&target_rel_normalized).join(rel_to_pattern);

                    if is_ignored(ignore_matcher, &target_path) {
                        continue;
                    }

                    on_entry(file_source, target_path, pattern_key.clone())?;
                }
            } else {
                let target_path = target_dir.join(&target_rel_normalized);

                if is_ignored(ignore_matcher, &target_path) {
                    continue;
                }

                on_entry(source_path, target_path, pattern_key)?;
            }
        }
    }

    Ok(())
}
