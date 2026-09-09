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
        Err(error) => Err(miette!(error))
            .wrap_err_with(|| format!("cannot inspect source path `{}`", path.display())),
    }
}

fn dir_identity(path: &Path) -> Result<DirIdentity> {
    let metadata = fs::metadata(path).map_err(|error| miette!(error))?;
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
        let entry = entry.map_err(|error| miette!(error))?;
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
            fs::write(t.join("vimrc"), b"set").unwrap();
            portal_map!("vimrc" => ".vimrc")
        } => resolved_list!("vimrc" => ".vimrc");
        "literal_file_maps_to_exact_target"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("a")).unwrap();
            fs::create_dir_all(t.join("sub/a")).unwrap();
            fs::write(t.join("a/b"), b"a").unwrap();
            fs::write(t.join("sub/a/b"), b"b").unwrap();
            portal_map!("a/b" => "dir")
        } => resolved_list!(
            "a/b" => "dir",
        );
        "literal_portal_is_anchored_does_not_match_nested_path"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("vimrc"), b"set").unwrap();
            portal_map!("./vimrc" => "./.vimrc")
        } => resolved_list!("vimrc" => ".vimrc");
        "dot_slash_prefix_is_stripped"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("nvim/lua")).unwrap();
            fs::write(t.join("nvim/init.lua"), b"-- lua").unwrap();
            fs::write(t.join("nvim/lua/mappings.lua"), b"-- mappings").unwrap();
            portal_map!("nvim" => ".config/nvim")
        } => resolved_list!(
            "nvim/init.lua" => ".config/nvim/init.lua",
            "nvim/lua/mappings.lua" => ".config/nvim/lua/mappings.lua"
        );
        "directory_portal_appends_relative_suffix"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("config/sub")).unwrap();
            fs::write(t.join("config/one.toml"), b"a").unwrap();
            fs::write(t.join("config/sub/two.toml"), b"b").unwrap();
            portal_map!("config/*.toml" => ".config")
        } => resolved_list!(
            "config/one.toml" => ".config/one.toml"
        );
        "wildcard_pattern_does_not_cross_directory"
    )]
    #[test_case(
        |t| {
            fs::create_dir_all(t.join("config/sub")).unwrap();
            fs::write(t.join("config/one.toml"), b"a").unwrap();
            fs::write(t.join("config/sub/two.toml"), b"b").unwrap();
            portal_map!("config/**/*.toml" => ".config")
        } => resolved_list!(
            "config/one.toml" => ".config/one.toml",
            "config/sub/two.toml" => ".config/sub/two.toml"
        );
        "recursive_wildcard_pattern_appends_remainder"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("empty")).unwrap();
            portal_map!("empty" => ".config/empty")
        } => resolved_list!();
        "empty_directory_expands_to_nothing"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("real"), b"content").unwrap();
            std::os::unix::fs::symlink(t.join("real"), t.join("link")).unwrap();
            portal_map!("link" => ".link")
        } => resolved_list!("link" => ".link");
        "symlink_to_file_maps_to_exact_target"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("real")).unwrap();
            fs::write(t.join("real/a"), b"a").unwrap();
            fs::write(t.join("real/b"), b"b").unwrap();
            std::os::unix::fs::symlink(t.join("real"), t.join("dirlink")).unwrap();
            portal_map!("dirlink" => ".config")
        } => resolved_list!(
            "dirlink/a" => ".config/a",
            "dirlink/b" => ".config/b"
        );
        "symlink_to_directory_maps_contents"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("data"), b"content").unwrap();
            std::os::unix::fs::symlink(t.join("data"), t.join("link.lnk")).unwrap();
            portal_map!("*.lnk" => ".dots")
        } => resolved_list!("link.lnk" => ".dots/link.lnk");
        "symlink_file_matched_by_wildcard"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("a.conf"), b"a").unwrap();
            portal_map!("*.conf" => ".")
        } => resolved_list!("a.conf" => "a.conf");
        "glob_root_destination_normalizes_to_plain_target"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("dir")).unwrap();
            fs::write(t.join("dir/a"), b"a").unwrap();
            portal_map!("dir" => ".")
        } => resolved_list!("dir/a" => "a");
        "literal_directory_root_destination_normalizes_to_plain_target"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("a.txt"), b"a").unwrap();
            portal_map!("a.txt" => ".")
        } => panics "cannot target `.`";
        "literal_file_root_destination_is_rejected"
    )]
    #[test_case(|_t| portal_map!("" => ".vimrc") => panics "invalid portal source path"; "empty_source_is_rejected")]
    #[test_case(|_t| portal_map!("vimrc" => "/home/.vimrc") => panics "invalid portal target path"; "absolute_target_is_rejected")]
    #[test_case(|_t| portal_map!("a//b" => ".a") => panics "invalid portal source path"; "double_slash_in_source_is_rejected")]
    #[test_case(|_t| portal_map!("sub/" => ".sub") => panics "invalid portal source path"; "trailing_slash_in_source_is_rejected")]
    #[test_case(|_t| portal_map!("a" => ".b//c") => panics "invalid portal target path"; "double_slash_in_target_is_rejected")]
    #[test_case(|_t| portal_map!("a" => ".sub/") => panics "invalid portal target path"; "trailing_slash_in_target_is_rejected")]
    #[test_case(|_t| portal_map!("a/../vimrc" => ".vimrc") => panics "a/../vimrc"; "parent_component_in_source_is_rejected")]
    #[test_case(|_t| portal_map!("{a,b}" => ".a") => panics "in portal source"; "brace_expansion_in_source_is_rejected")]
    #[test_case(|_t| portal_map!("a" => ".{a,b}") => panics "in portal target"; "brace_expansion_in_target_is_rejected")]
    #[test_case(|_t| portal_map!("a" => "*.conf") => panics "cannot contain glob syntax"; "wildcard_target_is_rejected")]
    #[test_case(|_t| portal_map!("missing" => ".missing") => panics "does not exist"; "missing_literal_source_is_rejected")]
    #[test_case(
        |t| {
            std::os::unix::fs::symlink(t.join("nowhere"), t.join("link")).unwrap();
            portal_map!("link" => "link")
        } => panics "dangling symlink";
        "dangling_literal_source_is_rejected"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("dir")).unwrap();
            fs::write(t.join("dir/ok"), b"ok").unwrap();
            std::os::unix::fs::symlink(t.join("nowhere"), t.join("dir/broken")).unwrap();
            portal_map!("dir" => ".config")
        } => panics "dangling symlink";
        "dangling_symlink_within_directory_is_rejected"
    )]
    #[test_case(
        |t| {
            fs::write(t.join("ok"), b"ok").unwrap();
            std::os::unix::fs::symlink(t.join("nowhere"), t.join("broken")).unwrap();
            portal_map!("*" => ".dots")
        } => panics "dangling symlink";
        "dangling_symlink_matched_by_wildcard_is_rejected"
    )]
    #[test_case(
        |t| {
            std::os::unix::fs::symlink(t.join("a"), t.join("a")).unwrap();
            portal_map!("a" => ".a")
        } => panics "cannot inspect source path";
        "self_referential_symlink_is_rejected"
    )]
    #[test_case(
        |t| {
            std::os::unix::fs::symlink(t.join("b"), t.join("a")).unwrap();
            std::os::unix::fs::symlink(t.join("a"), t.join("b")).unwrap();
            portal_map!("a" => ".a")
        } => panics "cannot inspect source path";
        "mutual_symlink_cycle_is_rejected"
    )]
    #[test_case(
        |t| {
            fs::create_dir(t.join("dir")).unwrap();
            fs::write(t.join("dir/real"), b"x").unwrap();
            std::os::unix::fs::symlink(t.join("dir"), t.join("dir/loop")).unwrap();
            portal_map!("dir" => ".config")
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
