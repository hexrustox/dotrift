use std::collections::{BTreeMap, HashMap};
use std::path::{Component, PathBuf};

use color_eyre::eyre::{Result, eyre};

use crate::command::apply::PortalEntry;

#[derive(Debug)]
pub enum Node {
    File(PortalEntry),
    Dir(BTreeMap<String, Node>),
}

impl Default for Node {
    fn default() -> Self {
        Node::Dir(BTreeMap::new())
    }
}

pub fn build_tree(entries: HashMap<PathBuf, PortalEntry>) -> Result<Node> {
    let mut root = Node::default();

    for (target_path, entry) in entries {
        insert_entry(&mut root, target_path, entry)?;
    }

    Ok(root)
}

fn insert_entry(root: &mut Node, target_path: PathBuf, entry: PortalEntry) -> Result<()> {
    let mut components: Vec<_> = target_path.components().collect();

    if components.is_empty() {
        return Err(eyre!("Cannot insert empty target path"));
    }

    if let Some(Component::RootDir) = components.first() {
        components = components[1..].to_vec();
    }

    let mut current = root;
    let count = components.len();

    for (i, component) in components.iter().enumerate() {
        let name = component.as_os_str().to_string_lossy().to_string();
        let is_last = i == count - 1;

        match current {
            Node::Dir(children) => {
                if is_last {
                    if let Some(existing) = children.get(&name) {
                        match existing {
                            Node::File { .. } => {
                                return Err(eyre!(
                                    "File already exists at `{}`",
                                    target_path.display()
                                ));
                            }
                            Node::Dir(_) => {
                                return Err(eyre!(
                                    "Directory exists when creating file at `{}`",
                                    target_path.display()
                                ));
                            }
                        }
                    }
                    children.insert(name, Node::File(entry));
                    return Ok(());
                }

                let child = children.entry(name).or_default();
                current = child;
            }
            Node::File { .. } => {
                return Err(eyre!(
                    "File exists when creating directory at `{}`",
                    target_path.display()
                ));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    macro_rules! portal_entries {
        ($($p:literal),*) => {
            crate::portal_entries!($(("", $p)),*)
        };
    }

    fn node_count(node: &Node) -> usize {
        match node {
            Node::File(_) => 1,
            Node::Dir(children) => 1 + children.values().map(node_count).sum::<usize>(),
        }
    }

    fn find_file<'a>(node: &'a Node, path: &[&str]) -> Option<&'a PortalEntry> {
        let mut current = node;
        for segment in path {
            match current {
                Node::Dir(children) => {
                    current = children.get(*segment)?;
                }
                Node::File(_) => return None,
            }
        }
        match current {
            Node::File(entry) => Some(entry),
            Node::Dir(_) => None,
        }
    }

    #[test_case(portal_entries!("/a.txt"), &[&["a.txt"]], 1; "single_file_at_root")]
    #[test_case(portal_entries!("/a/b.txt"), &[&["a", "b.txt"]], 2; "single_file_nested")]
    #[test_case(portal_entries!("/dir/a.txt", "/dir/b.txt"), &[&["dir", "a.txt"], &["dir", "b.txt"]], 3; "multiple_files_same_dir")]
    fn test_build_tree(
        entries: HashMap<PathBuf, PortalEntry>,
        assertions: &[&[&str]],
        total: usize,
    ) {
        let tree = build_tree(entries).unwrap();

        for a in assertions {
            find_file(&tree, a).unwrap();
        }
        assert_eq!(node_count(&tree), total + 1);
    }

    #[test_case("/file", "/file" => panics "File already exist"; "same_file")]
    #[test_case("/dir", "/dir/file" => panics "File exist"; "file")]
    #[test_case("/dir/file", "/dir" => panics "Directory exist"; "directory")]
    fn test_conflict(e1: &str, e2: &str) {
        let mut t = Node::default();
        insert_entry(&mut t, e1.into(), Default::default()).unwrap();
        insert_entry(&mut t, e2.into(), Default::default()).unwrap();
    }
}
