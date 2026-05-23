use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};

use color_eyre::eyre::{Result, eyre};

use crate::command::apply::PortalEntry;

#[derive(Debug)]
pub enum Node {
    File(PortalEntry),
    Marked(String),
    Dir(BTreeMap<String, Node>),
}

impl Default for Node {
    fn default() -> Self {
        Node::Dir(BTreeMap::new())
    }
}

impl Node {
    fn path_components(path: &Path) -> Result<Vec<String>> {
        let mut comps: Vec<_> = path.components().collect();
        if comps.is_empty() {
            return Err(eyre!("Cannot insert empty target path"));
        }
        if let Some(Component::RootDir) = comps.first() {
            comps = comps[1..].to_vec();
        }
        Ok(comps
            .into_iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect())
    }

    fn traverse_and_insert(&mut self, path: &Path, node: Node) -> Result<Option<String>> {
        let components = Self::path_components(path)?;

        let count = components.len();
        let mut current = self;

        for (i, name) in components.iter().enumerate() {
            let is_last = i == count - 1;

            match current {
                Node::Dir(children) => {
                    if is_last {
                        if let Some(existing) = children.get(name) {
                            match existing {
                                Node::Dir(_) => {
                                    return Err(eyre!(
                                        "Directory exists when creating file at `{}`",
                                        path.display()
                                    ));
                                }
                                Node::File(_) => {
                                    return Err(eyre!(
                                        "File already exists at `{}`",
                                        path.display()
                                    ));
                                }
                                Node::Marked(key) => {
                                    return Ok(Some(key.clone()));
                                }
                            }
                        }
                        children.insert(name.clone(), node);
                        return Ok(None);
                    }
                    let child = children.entry(name.clone()).or_default();
                    current = child;
                }
                Node::File { .. } => {
                    return Err(eyre!(
                        "File exists when creating directory at `{}`",
                        path.display()
                    ));
                }
                Node::Marked(key) => {
                    return Ok(Some(key.clone()));
                }
            }
        }

        Ok(None)
    }

    fn insert_entry(&mut self, target_path: PathBuf, entry: PortalEntry) -> Result<()> {
        self.traverse_and_insert(&target_path, Node::File(entry))?;
        Ok(())
    }

    pub fn check_entry(&mut self, path: &Path, key: String) -> Result<Option<String>> {
        self.traverse_and_insert(path, Node::Marked(key))
    }

    fn all_keys(&self) -> Vec<String> {
        match self {
            Node::Marked(k) => vec![k.clone()],
            Node::Dir(children) => {
                let mut keys = Vec::new();
                for child in children.values() {
                    keys.extend(child.all_keys());
                }
                keys
            }
            Node::File(_) => Vec::new(),
        }
    }

    pub fn key_at(&self, path: &Path) -> Vec<String> {
        let components = match Self::path_components(path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let mut current = self;
        for name in &components {
            match current {
                Node::Dir(children) => {
                    current = match children.get(name) {
                        Some(child) => child,
                        None => return Vec::new(),
                    };
                }
                _ => return current.all_keys(),
            }
        }
        current.all_keys()
    }
}

pub fn build_tree(entries: HashMap<PathBuf, PortalEntry>) -> Result<Node> {
    let mut root = Node::default();

    for (target_path, entry) in entries {
        root.insert_entry(target_path, entry)?;
    }

    Ok(root)
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
            Node::Marked(_) => 1,
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
                Node::File(_) | Node::Marked(_) => return None,
            }
        }
        match current {
            Node::File(entry) => Some(entry),
            Node::Marked(_) | Node::Dir(_) => None,
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

    #[test_case("/dir", "/dir/file" => panics "File exist"; "file")]
    #[test_case("/dir/file", "/dir" => panics "Directory exist"; "directory")]
    fn test_conflict(e1: &str, e2: &str) {
        let mut t = Node::default();
        t.insert_entry(e1.into(), Default::default()).unwrap();
        t.insert_entry(e2.into(), Default::default()).unwrap();
    }
}
