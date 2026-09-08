use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
};

use miette::{Result, miette};

use super::portals::ResolvedPortal;

pub(super) fn validate_targets(entries: &[ResolvedPortal]) -> Result<()> {
    let mut tree = Node::Dir(BTreeMap::new());
    for entry in entries {
        tree.insert(&entry.target, &entry.source)?;
    }
    Ok(())
}

enum Node {
    Dir(BTreeMap<OsString, Node>),
    File(PathBuf),
}

impl Default for Node {
    fn default() -> Self {
        Self::Dir(BTreeMap::new())
    }
}

impl Node {
    fn insert(&mut self, target: &Path, source: &Path) -> Result<()> {
        let mut node = self;
        let mut consumed = PathBuf::new();
        let mut components = target.components().peekable();
        while let Some(component) = components.next() {
            let name = component.as_os_str().to_owned();
            match node {
                Node::File(_) => {
                    return Err(miette!(
                        "structural conflict between `{}` and `{}`",
                        consumed.display(),
                        target.display()
                    ));
                }
                Node::Dir(children) => {
                    consumed.push(&name);
                    let child = children.entry(name).or_default();
                    if components.peek().is_none() {
                        if let Node::File(existing) = child {
                            return Err(miette!(
                                "collision at `{}` between `{}` and `{}`",
                                target.display(),
                                existing.display(),
                                source.display()
                            ));
                        }
                        if matches!(child, Node::Dir(children) if children.is_empty()) {
                            *child = Node::File(source.to_path_buf());
                            return Ok(());
                        }
                        let descendant = consumed.join(child.first_file_path());
                        return Err(miette!(
                            "structural conflict between `{}` and `{}`",
                            target.display(),
                            descendant.display()
                        ));
                    }
                    node = child;
                }
            }
        }
        unreachable!("validated targets are non-empty relative paths")
    }

    fn first_file_path(&self) -> PathBuf {
        match self {
            Node::File(_) => PathBuf::new(),
            Node::Dir(children) => {
                let (name, child) = children.iter().next().unwrap_or_else(|| {
                    unreachable!("caller guarantees the directory has children")
                });
                PathBuf::from(name).join(child.first_file_path())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use crate::config::portals::resolved_list;

    use super::*;

    #[test_case(resolved_list!(); "empty_target_set_is_valid")]
    #[test_case(resolved_list!("a" => ".config/a") ; "single_distinct_target_is_valid")]
    #[test_case(resolved_list!("a" => ".config/a", "b" => ".config/b") ; "distinct_targets_under_shared_directory")]
    #[test_case(resolved_list!("a" => "x/y", "b" => "x/z") ; "sibling_targets_under_nested_directory")]
    #[test_case(resolved_list!("a" => "deep/a/b/c", "b" => "deep/d/e") ; "deeply_nested_distinct_targets")]
    #[test_case(resolved_list!("a" => ".config", "b" => ".config") => panics "collision at"; "identical_targets_collide")]
    #[test_case(resolved_list!("a" => "x", "b" => "x/y") => panics "structural conflict"; "file_target_blocked_by_nested_target")]
    #[test_case(resolved_list!("a" => "x/y", "b" => "x") => panics "structural conflict"; "nested_target_blocked_by_file_target")]
    #[test_case(resolved_list!("a" => "x/y/z", "b" => "x/y") => panics "structural conflict"; "deeply_nested_target_blocked_by_file_target")]
    fn validates_targets_are_structurally_disjoint(entries: Vec<ResolvedPortal>) {
        validate_targets(&entries).unwrap()
    }
}
