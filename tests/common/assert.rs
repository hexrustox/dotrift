use std::path::Path;

use super::env::TestEnv;

/// Looks up the state record for `path`, panicking on DB errors.
pub fn record_of(env: &TestEnv, path: &Path) -> Option<dotrift::state::StateRecord> {
    env.database().record(path).unwrap()
}

/// Asserts that some cause in `error`'s chain contains `needle`.
pub fn assert_error_chain(error: &miette::Report, needle: &str) {
    assert!(
        error
            .chain()
            .any(|cause| cause.to_string().contains(needle)),
        "expected an error containing `{needle}` but got: {error:?}"
    );
}

/// `insta` settings that filter this env's temp root out of snapshot output.
///
/// `assert_snapshot!` must still be invoked from the test file (not from a
/// shared helper) so insta derives the correct snapshot name and `tests/`
/// directory from the calling module.
pub fn snapshot_settings(env: &TestEnv) -> insta::Settings {
    let mut settings = insta::Settings::new();
    settings.add_filter(env.root().to_str().unwrap(), "<root>");
    settings
}

/// The current test's generated name, stable under `#[test_case]` labels.
pub fn test_name() -> String {
    std::thread::current().name().unwrap().replace(":", "_")
}
