mod common;

use std::fs;

use dotrift::commands::apply::ApplyOptions;

use test_case::test_case;

use common::{ApplyScenario, snapshot_settings, test_name};

#[derive(Debug, Clone, Copy)]
enum PrintScenario {
    Fresh,
    CleanUp,
    Prune,
}

fn captured_with_options(scenario: &ApplyScenario, options: ApplyOptions) -> String {
    dotrift::capture::clear();
    scenario
        .try_run_with_options(options)
        .expect("apply failed");
    dotrift::capture::take()
}

fn scenario_for(scenario: PrintScenario) -> ApplyScenario {
    match scenario {
        PrintScenario::Fresh => ApplyScenario::new(|source, _target| {
            fs::write(source.join("a.txt"), b"A").unwrap();
            fs::write(source.join("b.txt"), b"B").unwrap();
            "[portal]\n\"a.txt\" = \"a.txt\"\n\"b.txt\" = \"b.txt\"\n"
        }),
        PrintScenario::CleanUp => {
            let scenario = ApplyScenario::new(|source, _target| {
                fs::write(source.join("keep.txt"), b"keep").unwrap();
                fs::write(source.join("remove.txt"), b"remove").unwrap();
                "[portal]\n\"keep.txt\" = \"keep.txt\"\n\"remove.txt\" = \"remove.txt\"\n"
            });
            scenario.run();
            scenario.write_config("[portal]\n\"keep.txt\" = \"keep.txt\"\n");
            scenario
        }
        PrintScenario::Prune => {
            let scenario = ApplyScenario::new(|source, _target| {
                fs::write(source.join("file.txt"), b"A").unwrap();
                "[portal]\n\"file.txt\" = \"a/b/file.txt\"\n"
            });
            scenario.run();
            scenario.write_config("[portal]\n");
            scenario
        }
    }
}

fn options_for(scenario: PrintScenario, verbose: bool) -> ApplyOptions {
    match scenario {
        PrintScenario::Fresh => {
            if verbose {
                ApplyOptions {
                    verbose: true,
                    ..Default::default()
                }
            } else {
                ApplyOptions::default()
            }
        }
        PrintScenario::CleanUp => {
            if verbose {
                ApplyOptions {
                    verbose: true,
                    clean_up: true,
                    ..Default::default()
                }
            } else {
                ApplyOptions {
                    clean_up: true,
                    ..Default::default()
                }
            }
        }
        PrintScenario::Prune => {
            if verbose {
                ApplyOptions {
                    verbose: true,
                    clean_up: true,
                    prune_empty_dirs: true,
                    ..Default::default()
                }
            } else {
                ApplyOptions {
                    clean_up: true,
                    prune_empty_dirs: true,
                    ..Default::default()
                }
            }
        }
    }
}

fn quiet_options_for(scenario: PrintScenario) -> ApplyOptions {
    match scenario {
        PrintScenario::Fresh => ApplyOptions {
            quiet: true,
            ..Default::default()
        },
        PrintScenario::CleanUp => ApplyOptions {
            quiet: true,
            clean_up: true,
            ..Default::default()
        },
        PrintScenario::Prune => ApplyOptions {
            quiet: true,
            clean_up: true,
            prune_empty_dirs: true,
            ..Default::default()
        },
    }
}

#[test_case(PrintScenario::Fresh, true; "fresh_verbose_prints_per_path_and_summary")]
#[test_case(PrintScenario::Fresh, false; "fresh_default_prints_only_summary")]
#[test_case(PrintScenario::CleanUp, true; "clean_up_verbose_reports_removed_and_summary")]
#[test_case(PrintScenario::CleanUp, false; "clean_up_default_reports_summary")]
#[test_case(PrintScenario::Prune, true; "prune_verbose_reports_pruned_parents")]
#[test_case(PrintScenario::Prune, false; "prune_default_reports_summary")]
fn snapshot_prints_output(scenario: PrintScenario, verbose: bool) {
    let prepared = scenario_for(scenario);
    let options = options_for(scenario, verbose);
    let captured = captured_with_options(&prepared, options);
    snapshot_settings(&prepared.env).bind(|| {
        insta::assert_snapshot!(test_name(), captured);
    });
}

#[test_case(PrintScenario::Fresh; "fresh_quiet_suppresses_output")]
#[test_case(PrintScenario::CleanUp; "clean_up_quiet_suppresses_output")]
#[test_case(PrintScenario::Prune; "prune_quiet_suppresses_output")]
fn quiet_produces_no_output(scenario: PrintScenario) {
    let prepared = scenario_for(scenario);
    let options = quiet_options_for(scenario);
    let captured = captured_with_options(&prepared, options);
    assert_eq!(captured, "");
}
