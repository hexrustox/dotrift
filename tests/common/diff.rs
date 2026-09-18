use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use super::env::TestEnv;
use super::pager::config_table_toml;

/// A diff-command capture script that records its arguments, one per line,
/// and prints nothing to standard output.
pub fn argv_diff_script(env: &TestEnv) -> (PathBuf, PathBuf) {
    let output = env.path("diff-argv.txt");
    let script = env.path("capture-diff-argv.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nfor arg in \"$@\"; do printf '%s\\n' \"$arg\" >> {}; done\n",
            output.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    (script, output)
}

/// A diff-command script with the given POSIX sh body. Name it with the
/// generic fixture scheme (`script1`, `script2`); the body's role lives in
/// the test name.
pub fn diff_script(env: &TestEnv, name: &str, body: &str) -> PathBuf {
    let script = env.path(format!("diff-{name}.sh"));
    fs::write(&script, format!("#!/bin/sh\n{body}")).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

pub fn config_diff_toml(command: &std::path::Path, args: &[&str]) -> String {
    config_table_toml("diff", command, args)
}
