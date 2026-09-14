mod common;

use std::{fs, path::Path};

use common::{
    Action, ApplyScenario, assert_symlink_tree, cleanup_world_strategy, count_files_on_disk,
    dry_run_output, file_count, materialize, render_portals, world_strategy,
};
use dotrift::commands::apply::ApplyOptions;
use proptest::prelude::*;

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
