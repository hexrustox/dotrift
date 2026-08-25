mod common;

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fmt::Debug,
    fs,
    path::{Path, PathBuf},
};

use dotrift::commands::apply::ApplyOptions;
use dotrift::state::{Kind, StateDatabase};
use proptest::prelude::*;

use crate::common::ApplyScenario;

#[derive(Debug, Clone)]
enum Node<T> {
    File(T),
    Dir(BTreeMap<OsString, Node<T>>),
}

const MAX_DIR_DEPTH: usize = 2;

fn name_strategy() -> impl Strategy<Value = OsString> {
    let letter = prop_oneof![prop::char::range('a', 'z'), prop::char::range('A', 'Z'),];
    prop::collection::vec(letter, 1..=5)
        .prop_map(|chars| OsString::from(chars.into_iter().collect::<String>()))
}

fn source_payload() -> BoxedStrategy<Vec<u8>> {
    prop::collection::vec(any::<u8>(), 64).boxed()
}

fn node_strategy<T>(
    payload: BoxedStrategy<T>,
    dir_levels_remaining: usize,
) -> BoxedStrategy<Node<T>>
where
    T: Debug + Clone + 'static,
{
    if dir_levels_remaining == 0 {
        payload.prop_map(Node::File).boxed()
    } else {
        prop_oneof![
            payload.clone().prop_map(Node::File),
            dir_strategy(payload, dir_levels_remaining - 1),
        ]
        .boxed()
    }
}

fn dir_strategy<T>(payload: BoxedStrategy<T>, dir_levels_remaining: usize) -> BoxedStrategy<Node<T>>
where
    T: Debug + Clone + 'static,
{
    prop::collection::btree_map(
        name_strategy(),
        node_strategy(payload, dir_levels_remaining),
        4..=8,
    )
    .prop_map(Node::Dir)
    .boxed()
}

fn source_tree_strategy() -> BoxedStrategy<Node<Vec<u8>>> {
    dir_strategy(source_payload(), MAX_DIR_DEPTH)
}

fn collect_files<T>(node: &Node<T>, base: &Path, out: &mut Vec<PathBuf>) {
    match node {
        Node::File(_) => out.push(base.to_path_buf()),
        Node::Dir(children) => {
            for (name, child) in children {
                collect_files(child, &base.join(name), out);
            }
        }
    }
}

fn file_count<T>(node: &Node<T>) -> usize {
    match node {
        Node::File(_) => 1,
        Node::Dir(children) => children.values().map(file_count).sum(),
    }
}

fn collect_source_paths(node: &Node<PathBuf>, out: &mut Vec<PathBuf>) {
    match node {
        Node::File(payload) => out.push(payload.clone()),
        Node::Dir(children) => {
            for child in children.values() {
                collect_source_paths(child, out);
            }
        }
    }
}

fn filter_files(
    node: Node<PathBuf>,
    keep: &mut impl FnMut(&PathBuf) -> bool,
) -> Option<Node<PathBuf>> {
    match node {
        Node::File(payload) => keep(&payload).then_some(Node::File(payload)),
        Node::Dir(children) => {
            let filtered: BTreeMap<OsString, Node<PathBuf>> = children
                .into_iter()
                .filter_map(|(name, child)| filter_files(child, keep).map(|child| (name, child)))
                .collect();
            if filtered.is_empty() {
                None
            } else {
                Some(Node::Dir(filtered))
            }
        }
    }
}

fn count_files_on_disk(root: &Path) -> usize {
    let mut n = 0;
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            n += count_files_on_disk(&path);
        } else {
            n += 1;
        }
    }
    n
}

fn reassign(node: Node<()>, payloads: &mut std::collections::VecDeque<PathBuf>) -> Node<PathBuf> {
    match node {
        Node::File(()) => Node::File(payloads.pop_front().expect("payloads match file count")),
        Node::Dir(children) => Node::Dir(
            children
                .into_iter()
                .map(|(name, child)| (name, reassign(child, payloads)))
                .collect(),
        ),
    }
}

fn shuffle<T>(items: &mut [T], mut seed: u64) {
    for i in (1..items.len()).rev() {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let j = (seed >> 33) as usize % (i + 1);
        items.swap(i, j);
    }
}

fn world_strategy() -> impl Strategy<Value = (Node<Vec<u8>>, Node<PathBuf>)> {
    source_tree_strategy().prop_flat_map(|source| {
        let mut files = Vec::new();
        collect_files(&source, Path::new(""), &mut files);

        let target = dir_strategy(proptest::strategy::Just(()).boxed(), MAX_DIR_DEPTH)
            .prop_filter("target has more files than source", {
                let len = files.len();
                move |structure| file_count(structure) <= len
            })
            .prop_flat_map(move |structure| (proptest::strategy::Just(structure), any::<u64>()))
            .prop_map(move |(structure, seed)| {
                let mut shuffled = files.clone();
                shuffle(&mut shuffled, seed);
                let mut payloads: std::collections::VecDeque<_> =
                    shuffled.into_iter().take(file_count(&structure)).collect();
                reassign(structure, &mut payloads)
            });

        (proptest::strategy::Just(source), target)
    })
}

fn cleanup_world_strategy() -> impl Strategy<Value = (Node<Vec<u8>>, Node<PathBuf>, Node<PathBuf>)>
{
    world_strategy().prop_flat_map(|(source, target)| {
        let mut leaves = Vec::new();
        collect_source_paths(&target, &mut leaves);
        let len = leaves.len();
        let target_for_mask = target.clone();
        prop::collection::vec(any::<bool>(), len)
            .prop_filter("removes at least one target", |keep| {
                keep.iter().any(|keep| !keep)
            })
            .prop_flat_map(move |keep| {
                let mut position = 0;
                let pruned = filter_files(target_for_mask.clone(), &mut |_| {
                    let keep_here = keep[position];
                    position += 1;
                    keep_here
                })
                .unwrap_or(Node::Dir(BTreeMap::new()));
                proptest::strategy::Just((source.clone(), target.clone(), pruned))
            })
    })
}

fn build_config(node: &Node<PathBuf>, base: &Path, out: &mut String) {
    match node {
        Node::File(source) => {
            out.push_str(&format!(
                "\n\"{}\" = \"{}\"",
                source.display(),
                base.display()
            ));
        }
        Node::Dir(children) => {
            for (name, child) in children {
                build_config(child, &base.join(name), out);
            }
        }
    }
}

fn render_portals(target: &Node<PathBuf>) -> String {
    let mut out = "[portal]".to_string();
    build_config(target, Path::new(""), &mut out);
    out
}

fn write_node(root: &Path, node: &Node<Vec<u8>>) {
    match node {
        Node::File(bytes) => fs::write(root, bytes).unwrap(),
        Node::Dir(children) => {
            fs::create_dir_all(root).unwrap();
            for (name, child) in children {
                write_node(&root.join(name), child);
            }
        }
    }
}

fn materialize(source_dir: &Path, tree: &Node<Vec<u8>>) {
    write_node(source_dir, tree);
}

fn assert_symlink_tree(
    source: &Path,
    target: &Path,
    db: &StateDatabase,
    node: &Node<PathBuf>,
    base: &Path,
) -> Result<(), proptest::test_runner::TestCaseError> {
    match node {
        Node::File(payload) => {
            let link = target.join(base);
            let metadata = fs::symlink_metadata(&link).unwrap();
            prop_assert!(metadata.file_type().is_symlink());
            prop_assert_eq!(fs::read_link(&link).unwrap(), source.join(payload));
            let record = db
                .record(&link)
                .unwrap()
                .expect("no state record for deployed symlink");
            prop_assert_eq!(record.source_path, source.join(payload));
            prop_assert_eq!(record.kind, Kind::Symlink);
        }
        Node::Dir(children) => {
            prop_assert!(target.join(base).is_dir());
            for (name, child) in children {
                assert_symlink_tree(source, target, db, child, &base.join(name))?;
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Deployed,
    Replaced,
    Removed,
}

fn parse_actions(output: &str) -> Vec<(Action, PathBuf)> {
    output
        .lines()
        .map(|line| {
            let (verb, rest) = line.split_once(' ').expect("malformed dry-run line");
            let action = match verb {
                "deployed" => Action::Deployed,
                "replaced" => Action::Replaced,
                "removed" => Action::Removed,
                other => panic!("unexpected dry-run action `{other}`"),
            };
            // dry-run entries: "{action} {path} [{type}]" or "[{type} {mode}]"
            // `removed` has no suffix. Strip trailing " [...]" to recover path, preserving spaces.
            let path_str = if rest.ends_with(']') {
                if let Some((path_part, _)) = rest.rsplit_once(" [") {
                    path_part
                } else {
                    rest
                }
            } else {
                rest
            };
            (action, PathBuf::from(path_str))
        })
        .collect()
}

fn dry_run_output(scenario: &ApplyScenario, mut options: ApplyOptions) -> Vec<(Action, PathBuf)> {
    options.dry_run = true;
    dotrift::capture::clear();
    scenario.run_with_options(options);
    parse_actions(&dotrift::capture::take())
}

proptest! {
    #![proptest_config(proptest::test_runner::Config {
        cases: 1,
        ..proptest::test_runner::Config::default()
    })]

    #[test]
    fn apply_deploys_exact_symlink_tree((source_tree, target_tree) in world_strategy()) {
        let scenario = ApplyScenario::new(|_, _| "");
        materialize(&scenario.source, &source_tree);
        scenario.write_config(&render_portals(&target_tree));

        scenario.run();

        let db = scenario.env.database();
        assert_symlink_tree(&scenario.source, &scenario.target, &db, &target_tree, Path::new(""))?;
        prop_assert_eq!(count_files_on_disk(&scenario.target), file_count(&target_tree));
        prop_assert_eq!(db.managed_paths().unwrap().len(), file_count(&target_tree));
    }

    #[test]
    fn apply_cleanup_drops_removed_targets(
        (source_tree, target_tree, pruned_tree) in cleanup_world_strategy(),
    ) {
        let scenario = ApplyScenario::new(|_, _| "");
        materialize(&scenario.source, &source_tree);
        scenario.write_config(&render_portals(&target_tree));
        scenario.run();

        scenario.write_config(&render_portals(&pruned_tree));
        scenario.run_with_options(ApplyOptions {
            clean_up: true,
            ..Default::default()
        });

        let db = scenario.env.database();
        assert_symlink_tree(&scenario.source, &scenario.target, &db, &pruned_tree, Path::new(""))?;
        prop_assert_eq!(count_files_on_disk(&scenario.target), file_count(&pruned_tree));
        prop_assert_eq!(db.managed_paths().unwrap().len(), file_count(&pruned_tree));
    }

    #[test]
    fn apply_cleanup_prune_empty_dirs((source_tree, target_tree) in world_strategy()) {
        let scenario = ApplyScenario::new(|_, _| "");
        materialize(&scenario.source, &source_tree);
        scenario.write_config(&render_portals(&target_tree));
        scenario.run();

        scenario.write_config("");
        scenario.run_with_options(ApplyOptions {
            clean_up: true,
            prune_empty_dirs: true,
            ..Default::default()
        });

        let db = scenario.env.database();
        prop_assert_eq!(fs::read_dir(&scenario.target).unwrap().count(), 0);
        prop_assert_eq!(db.managed_paths().unwrap().len(), 0);
    }

    #[test]
    fn apply_dry_run_output_matches_real_deploy((source_tree, target_tree) in world_strategy()) {
        let scenario = ApplyScenario::new(|_, _| "");
        materialize(&scenario.source, &source_tree);
        scenario.write_config(&render_portals(&target_tree));

        let reported = dry_run_output(&scenario, ApplyOptions::default());

        prop_assert!(reported.iter().all(|(action, _)| *action == Action::Deployed));
        prop_assert_eq!(count_files_on_disk(&scenario.target), 0);

        scenario.run();

        for (_, path) in &reported {
            prop_assert!(fs::symlink_metadata(path).is_ok());
        }
        prop_assert_eq!(
            count_files_on_disk(&scenario.target),
            reported.len()
        );
    }

    #[test]
    fn apply_dry_run_cleanup_output_matches_removals(
        (source_tree, target_tree, pruned_tree) in cleanup_world_strategy(),
    ) {
        let scenario = ApplyScenario::new(|_, _| "");
        materialize(&scenario.source, &source_tree);
        scenario.write_config(&render_portals(&target_tree));
        scenario.run();

        scenario.write_config(&render_portals(&pruned_tree));

        let reported = dry_run_output(
            &scenario,
            ApplyOptions {
                clean_up: true,
                ..Default::default()
            },
        );

        prop_assert!(reported
            .iter()
            .all(|(action, _)| matches!(action, Action::Replaced | Action::Removed)));

        scenario.run_with_options(ApplyOptions {
            clean_up: true,
            ..Default::default()
        });

        for (action, path) in &reported {
            if *action == Action::Removed {
                prop_assert!(fs::symlink_metadata(path).is_err());
            } else {
                prop_assert!(fs::symlink_metadata(path).is_ok());
            }
        }
        prop_assert_eq!(
            count_files_on_disk(&scenario.target),
            reported
                .iter()
                .filter(|(action, _)| *action == Action::Replaced)
                .count()
        );
    }
}
