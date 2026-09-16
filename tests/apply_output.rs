mod common;

use std::fs;

use common::{ApplyScenario, snapshot_settings, test_name};
use dotrift::commands::apply::ApplyOptions;
use test_case::test_case;

/// Which output surface the run exercises: a plain deploy, a clean-up that
/// removes stale entries, or a clean-up that also prunes empty parents.
#[derive(Debug, Clone, Copy)]
enum OutputScenario {
    Fresh,
    CleanUp,
    Prune,
}

fn prepared_scenario(scenario: OutputScenario) -> ApplyScenario {
    match scenario {
        OutputScenario::Fresh => ApplyScenario::new(|source, _target| {
            fs::write(source.join("file1"), "content1").unwrap();
            fs::write(source.join("file2"), "content2").unwrap();
            r#"
[portal]
"file1" = "file1"
"file2" = "file2"
"#
        }),
        OutputScenario::CleanUp => {
            let scenario = ApplyScenario::new(|source, _target| {
                fs::write(source.join("file1"), "content1").unwrap();
                fs::write(source.join("file2"), "content2").unwrap();
                r#"
[portal]
"file1" = "file1"
"file2" = "file2"
"#
            });
            scenario.run();
            scenario.write_config(
                r#"
[portal]
"file1" = "file1"
"#,
            );
            scenario
        }
        OutputScenario::Prune => {
            let scenario = ApplyScenario::new(|source, _target| {
                fs::write(source.join("file1"), "content1").unwrap();
                r#"
[portal]
"file1" = "dir1/sub1/file1"
"#
            });
            scenario.run();
            scenario.write_config("[portal]\n");
            scenario
        }
    }
}

fn options_for(scenario: OutputScenario, verbose: bool) -> ApplyOptions {
    let base = match scenario {
        OutputScenario::Fresh => ApplyOptions::default(),
        OutputScenario::CleanUp => ApplyOptions {
            clean_up: true,
            ..Default::default()
        },
        OutputScenario::Prune => ApplyOptions {
            clean_up: true,
            prune_empty_dirs: true,
            ..Default::default()
        },
    };
    ApplyOptions { verbose, ..base }
}

fn capture(scenario: &ApplyScenario, options: ApplyOptions) -> String {
    dotrift::report::clear();
    scenario
        .try_run_with_options(options)
        .expect("apply failed");
    dotrift::report::take_output()
}

#[test_case(OutputScenario::Fresh, true; "fresh_verbose_prints_per_path_lines_and_summary")]
#[test_case(OutputScenario::Fresh, false; "fresh_default_prints_only_the_summary")]
#[test_case(OutputScenario::CleanUp, true; "clean_up_verbose_reports_removed_entries_and_summary")]
#[test_case(OutputScenario::CleanUp, false; "clean_up_default_reports_only_the_summary")]
#[test_case(OutputScenario::Prune, true; "prune_verbose_reports_pruned_parents_and_summary")]
#[test_case(OutputScenario::Prune, false; "prune_default_reports_only_the_summary")]
fn output_matches_snapshot(scenario: OutputScenario, verbose: bool) {
    let prepared = prepared_scenario(scenario);
    let captured = capture(&prepared, options_for(scenario, verbose));

    snapshot_settings(&prepared.env).bind(|| {
        insta::assert_snapshot!(test_name(), captured);
    });
}

#[test_case(OutputScenario::Fresh; "fresh_quiet_suppresses_all_output")]
#[test_case(OutputScenario::CleanUp; "clean_up_quiet_suppresses_all_output")]
#[test_case(OutputScenario::Prune; "prune_quiet_suppresses_all_output")]
fn quiet_mode_produces_no_output(scenario: OutputScenario) {
    let prepared = prepared_scenario(scenario);
    let captured = capture(
        &prepared,
        ApplyOptions {
            quiet: true,
            ..options_for(scenario, false)
        },
    );

    assert_eq!(captured, "");
}
