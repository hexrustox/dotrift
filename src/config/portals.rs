use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use glob::Pattern;
use miette::{Result, WrapErr, miette};

use super::{GLOB_MATCH_OPTIONS, ResolvedPortal, reject_brace_expansion, validate_relative};

pub(super) fn resolve_portals(
    source: &Path,
    portals: &BTreeMap<String, String>,
) -> Result<Vec<ResolvedPortal>> {
    let mut result = Vec::new();
    for (key, value) in portals {
        validate_relative(key, "portal source")?;
        validate_relative(value, "portal target")?;
        reject_brace_expansion(key, "portal source")?;
        reject_brace_expansion(value, "portal target")?;
        if contains_wildcard(value) {
            return Err(miette!(
                "portal target `{value}` cannot contain glob syntax"
            ));
        }
        let key = key.strip_prefix("./").unwrap_or(key);
        let value = value.strip_prefix("./").unwrap_or(value);
        if !contains_wildcard(key) {
            let path = source.join(key);
            if !path.exists() && fs::symlink_metadata(&path).is_err() {
                return Err(miette!("literal portal source `{key}` does not exist"));
            }
            match resolve_kind(&path)? {
                ResolvedKind::File => {
                    if value == "." {
                        return Err(miette!(
                            "literal file `{key}` cannot target `.`: the target would be the target-directory root"
                        ));
                    }
                    push_deployable(&mut result, &path, Path::new(value))?
                }
                ResolvedKind::Directory => {
                    let mut stack = Vec::new();
                    walk_following_links(&path, &mut stack, &mut |child, kind| match kind {
                        ResolvedKind::File => {
                            let relative = child.strip_prefix(&path).map_err(|_| {
                                miette!(
                                    "entry `{}` is outside the portal source `{}`",
                                    child.display(),
                                    path.display()
                                )
                            })?;
                            push_deployable(&mut result, child, &Path::new(value).join(relative))
                        }
                        ResolvedKind::Dangling => {
                            Err(miette!("dangling symlink `{}`", child.display()))
                        }
                        ResolvedKind::Special => Err(miette!(
                            "source path `{}` is not a regular file, symlink to a regular file, or directory",
                            child.display()
                        )),
                        ResolvedKind::Directory => {
                            unreachable!("directories are descended, never visited")
                        }
                    })?;
                }
                ResolvedKind::Dangling => {
                    return Err(miette!("dangling symlink `{}`", path.display()));
                }
                ResolvedKind::Special => {
                    return Err(miette!(
                        "source path `{}` is not a regular file, symlink to a regular file, or directory",
                        path.display()
                    ));
                }
            }
            continue;
        }

        let strip = wildcard_prefix(key);
        let pattern = Pattern::new(key)
            .map_err(|error| miette!("invalid portal pattern `{key}` because {error}"))?;
        let mut stack = Vec::new();
        walk_following_links(source, &mut stack, &mut |path, kind| {
            let relative = path.strip_prefix(source).map_err(|_| {
                miette!(
                    "entry `{}` is outside the source directory `{}`",
                    path.display(),
                    source.display()
                )
            })?;
            if !pattern.matches_path_with(relative, GLOB_MATCH_OPTIONS) {
                return Ok(());
            }
            match kind {
                ResolvedKind::File => {
                    let remainder = relative.strip_prefix(&strip).unwrap_or(relative);
                    push_deployable(&mut result, path, &Path::new(value).join(remainder))
                }
                ResolvedKind::Dangling => Err(miette!("dangling symlink `{}`", path.display())),
                ResolvedKind::Special => Err(miette!(
                    "source path `{}` is not a regular file, symlink to a regular file, or directory",
                    path.display()
                )),
                ResolvedKind::Directory => unreachable!("directories are descended, never visited"),
            }
        })?;
    }
    Ok(result)
}

fn push_deployable(result: &mut Vec<ResolvedPortal>, source: &Path, target: &Path) -> Result<()> {
    if !fs::metadata(source).is_ok_and(|meta| meta.is_file()) {
        return Err(miette!(
            "source path `{}` is not a regular file or symlink to a regular file",
            source.display()
        ));
    }
    result.push(ResolvedPortal {
        source: source.to_path_buf(),
        target: target.strip_prefix("./").unwrap_or(target).to_path_buf(),
    });
    Ok(())
}

fn contains_wildcard(value: &str) -> bool {
    value
        .bytes()
        .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b']'))
}

fn wildcard_prefix(pattern: &str) -> PathBuf {
    let mut prefix = PathBuf::new();
    for component in Path::new(pattern).components() {
        let value = component.as_os_str().to_string_lossy();
        if contains_wildcard(&value) {
            break;
        }
        prefix.push(value.as_ref());
    }
    prefix
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolvedKind {
    Directory,
    File,
    Dangling,
    Special,
}

type DirIdentity = (u64, u64);

fn resolve_kind(path: &Path) -> Result<ResolvedKind> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(ResolvedKind::Directory),
        Ok(metadata) if metadata.is_file() => Ok(ResolvedKind::File),
        Ok(_) => Ok(ResolvedKind::Special),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ResolvedKind::Dangling),
        Err(error) => {
            let cycle = error.raw_os_error() == Some(libc::ELOOP);
            let wrapped = Err::<ResolvedKind, miette::Report>(miette!(error))
                .wrap_err_with(|| format!("cannot inspect source path `{}`", path.display()));
            if cycle {
                Err(wrapped.expect_err("miette! is always Err")).wrap_err_with(|| {
                    format!(
                        "symlink cycle detected while inspecting `{}`",
                        path.display()
                    )
                })
            } else {
                wrapped.map(|_| ResolvedKind::Special)
            }
        }
    }
}

fn dir_identity(path: &Path) -> Result<DirIdentity> {
    let metadata = fs::metadata(path)
        .map_err(|error| miette!(error))
        .wrap_err_with(|| format!("cannot inspect source path `{}`", path.display()))?;
    Ok((metadata.dev(), metadata.ino()))
}

fn walk_following_links(
    root: &Path,
    stack: &mut Vec<DirIdentity>,
    visit: &mut dyn FnMut(&Path, ResolvedKind) -> Result<()>,
) -> Result<()> {
    let identity = dir_identity(root)?;
    if stack.contains(&identity) {
        return Err(miette!(
            "symlink cycle detected while traversing `{}`",
            root.display()
        ));
    }
    stack.push(identity);
    let read_dir = fs::read_dir(root)
        .map_err(|error| miette!(error))
        .wrap_err_with(|| format!("cannot read source directory `{}`", root.display()))?;
    for entry in read_dir {
        let entry = entry.map_err(|error| miette!(error)).wrap_err_with(|| {
            format!(
                "cannot list entries in source directory `{}`",
                root.display()
            )
        })?;
        let path = entry.path();
        match resolve_kind(&path)? {
            ResolvedKind::Directory => walk_following_links(&path, stack, visit)?,
            kind => visit(&path, kind)?,
        }
    }
    stack.pop();
    Ok(())
}

#[cfg(test)]
macro_rules! resolved_list {
    ($($source:literal => $target:literal),* $(,)?) => {
        {
            let vec: Vec<ResolvedPortal> = vec![
                $(ResolvedPortal {
                    source: PathBuf::from($source),
                    target: PathBuf::from($target),
                }),*
            ];
            vec
        }
    };
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use test_case::test_case;

    use super::*;

    macro_rules! portal_map {
        ($($source:literal => $target:literal),* $(,)?) => {
            BTreeMap::from([
                $(($source.to_string(), $target.to_string())),*
            ])
        };
    }

    #[test_case(|_t| portal_map!() => resolved_list!(); "empty_portals_produce_no_entries")]
    #[test_case(
        |t| {
            fs::write(t.join("file1"), b"content1").unwrap();
            portal_map!("file1" => "target1")
        } => resolved_list!("file1" => "target1");
        "literal_file_maps_to_exact_target"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("dir1/sub1")).unwrap();
            fs::write(t.join("dir1/file1"), b"content1").unwrap();
            fs::write(t.join("dir1/sub1/file1"), b"content2").unwrap();
            portal_map!("dir1/file1" => "dir2")
        } => resolved_list!(
            "dir1/file1" => "dir2",
        );
        "literal_portal_is_anchored_does_not_match_nested_path"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("file1"), b"content1").unwrap();
            portal_map!("./file1" => "./target1")
        } => resolved_list!("file1" => "target1");
        "dot_slash_prefix_is_stripped"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("dir1/sub1")).unwrap();
            fs::write(t.join("dir1/file1"), b"content1").unwrap();
            fs::write(t.join("dir1/sub1/file2"), b"content2").unwrap();
            portal_map!("dir1" => "dir2")
        } => resolved_list!(
            "dir1/file1" => "dir2/file1",
            "dir1/sub1/file2" => "dir2/sub1/file2"
        );
        "directory_portal_appends_relative_suffix"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("dir1/sub1")).unwrap();
            fs::write(t.join("dir1/file1.conf"), b"content1").unwrap();
            fs::write(t.join("dir1/sub1/file2"), b"content2").unwrap();
            portal_map!("dir1/*.conf" => "dir2")
        } => resolved_list!(
            "dir1/file1.conf" => "dir2/file1.conf"
        );
        "wildcard_pattern_does_not_cross_directory"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("dir1/sub1")).unwrap();
            fs::write(t.join("dir1/file1.conf"), b"content1").unwrap();
            fs::write(t.join("dir1/sub1/file2.conf"), b"content2").unwrap();
            portal_map!("dir1/**/*.conf" => "dir2")
        } => resolved_list!(
            "dir1/file1.conf" => "dir2/file1.conf",
            "dir1/sub1/file2.conf" => "dir2/sub1/file2.conf"
        );
        "recursive_wildcard_pattern_appends_remainder"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("dir1")).unwrap();
            portal_map!("dir1" => "dir2/dir1")
        } => resolved_list!();
        "empty_directory_expands_to_nothing"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("file1"), b"content1").unwrap();
            std::os::unix::fs::symlink(t.join("file1"), t.join("link1")).unwrap();
            portal_map!("link1" => "target1")
        } => resolved_list!("link1" => "target1");
        "symlink_to_file_maps_to_exact_target"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("dir1")).unwrap();
            fs::write(t.join("dir1/file1"), b"content1").unwrap();
            fs::write(t.join("dir1/file2"), b"content2").unwrap();
            std::os::unix::fs::symlink(t.join("dir1"), t.join("link1")).unwrap();
            portal_map!("link1" => "dir2")
        } => resolved_list!(
            "link1/file1" => "dir2/file1",
            "link1/file2" => "dir2/file2"
        );
        "symlink_to_directory_maps_contents"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("file1"), b"content1").unwrap();
            std::os::unix::fs::symlink(t.join("file1"), t.join("link1.lnk")).unwrap();
            portal_map!("*.lnk" => "dir2")
        } => resolved_list!("link1.lnk" => "dir2/link1.lnk");
        "symlink_file_matched_by_wildcard"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("file1.conf"), b"content1").unwrap();
            portal_map!("*.conf" => ".")
        } => resolved_list!("file1.conf" => "file1.conf");
        "glob_root_destination_normalizes_to_plain_target"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("dir1")).unwrap();
            fs::write(t.join("dir1/file1"), b"content1").unwrap();
            portal_map!("dir1" => ".")
        } => resolved_list!("dir1/file1" => "file1");
        "literal_directory_root_destination_normalizes_to_plain_target"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("file1"), b"content1").unwrap();
            portal_map!("file1" => ".")
        } => panics "cannot target `.`";
        "literal_file_root_destination_is_rejected"
    )]
    #[test_case(|_t| portal_map!("" => "target1") => panics "invalid portal source path"; "empty_source_is_rejected")]
    #[test_case(|_t| portal_map!("dir1//file1" => "target1") => panics "invalid portal source path"; "double_slash_in_source_is_rejected")]
    #[test_case(|_t| portal_map!("dir1/" => "target1") => panics "invalid portal source path"; "trailing_slash_in_source_is_rejected")]
    #[test_case(|_t| portal_map!("file1" => "/target") => panics "invalid portal target path"; "absolute_target_is_rejected")]
    #[test_case(|_t| portal_map!("file1" => "dir1//dir2") => panics "invalid portal target path"; "double_slash_in_target_is_rejected")]
    #[test_case(|_t| portal_map!("file1" => "dir1/") => panics "invalid portal target path"; "trailing_slash_in_target_is_rejected")]
    #[test_case(|_t| portal_map!("dir1/../file1" => "target1") => panics "dir1/../file1"; "parent_component_in_source_is_rejected")]
    #[test_case(|_t| portal_map!("{a,b}" => "dir1") => panics "in portal source"; "brace_expansion_in_source_is_rejected")]
    #[test_case(|_t| portal_map!("file1" => "{a,b}") => panics "in portal target"; "brace_expansion_in_target_is_rejected")]
    #[test_case(|_t| portal_map!("file1" => "*.conf") => panics "cannot contain glob syntax"; "wildcard_target_is_rejected")]
    #[test_case(|_t| portal_map!("file1" => "target1") => panics "does not exist"; "missing_literal_source_is_rejected")]
    #[test_case(
        |t| {
            std::os::unix::fs::symlink(t.join("link2"), t.join("link1")).unwrap();
            portal_map!("link1" => "link1")
        } => panics "dangling symlink";
        "dangling_literal_source_is_rejected"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("dir1")).unwrap();
            fs::write(t.join("dir1/file1"), b"content1").unwrap();
            std::os::unix::fs::symlink(t.join("link2"), t.join("dir1/link1")).unwrap();
            portal_map!("dir1" => "dir2")
        } => panics "dangling symlink";
        "dangling_symlink_within_directory_is_rejected"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("file1"), b"content1").unwrap();
            std::os::unix::fs::symlink(t.join("link2"), t.join("link1")).unwrap();
            portal_map!("*" => "dir1")
        } => panics "dangling symlink";
        "dangling_symlink_matched_by_wildcard_is_rejected"
    )]
    #[test_case(
        |t| {
            std::os::unix::fs::symlink(t.join("link1"), t.join("link1")).unwrap();
            portal_map!("link1" => "dir1")
        } => panics "symlink cycle detected";
        "self_referential_symlink_is_rejected"
    )]
    #[test_case(
        |t| {
            std::os::unix::fs::symlink(t.join("link2"), t.join("link1")).unwrap();
            std::os::unix::fs::symlink(t.join("link1"), t.join("link2")).unwrap();
            portal_map!("link1" => "dir1")
        } => panics "symlink cycle detected";
        "mutual_symlink_cycle_is_rejected"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("dir1")).unwrap();
            fs::write(t.join("dir1/file1"), b"content1").unwrap();
            std::os::unix::fs::symlink(t.join("dir1"), t.join("dir1/link1")).unwrap();
            portal_map!("dir1" => "dir2")
        } => panics "symlink cycle detected";
        "symlink_cycle_within_directory_is_rejected"
    )]
    fn expands_portals_to_resolved_entries(
        setup: impl Fn(&Path) -> BTreeMap<String, String>,
    ) -> Vec<ResolvedPortal> {
        let dir = tempdir().expect("cannot create temp dir");
        let mut entries = resolve_portals(dir.path(), &setup(dir.path()))
            .unwrap()
            .into_iter()
            .map(|entry| ResolvedPortal {
                source: entry.source.strip_prefix(dir.path()).unwrap().to_path_buf(),
                target: entry.target,
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.source.clone());
        entries
    }
}
