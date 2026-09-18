use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use super::env::TestEnv;

pub fn capture_script_named(env: &TestEnv, name: &str) -> (PathBuf, PathBuf) {
    let output = env.path(format!("{name}.txt"));
    let script = env.path(format!("capture-{name}.sh"));
    fs::write(&script, format!("#!/bin/sh\ncat > {}\n", output.display())).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    (script, output)
}

pub fn capture_script(env: &TestEnv) -> (PathBuf, PathBuf) {
    capture_script_named(env, "pager")
}

/// A pager capture script that also records its arguments, one per line,
/// before the diff.
pub fn argv_capture_script(env: &TestEnv) -> (PathBuf, PathBuf) {
    let output = env.path("pager-argv.txt");
    let script = env.path("capture-argv-pager.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nfor arg in \"$@\"; do printf '%s\\n' \"$arg\" >> {}; done\ncat >> {}\n",
            output.display(),
            output.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    (script, output)
}

pub(super) fn config_table_toml(table: &str, command: &std::path::Path, args: &[&str]) -> String {
    let args = args
        .iter()
        .map(|arg| format!("'{arg}'"))
        .collect::<Vec<_>>()
        .join(", ");
    if args.is_empty() {
        format!("[{table}]\ncommand = '{}'\n", command.display())
    } else {
        format!(
            "[{table}]\ncommand = '{}'\nargs = [{args}]\n",
            command.display()
        )
    }
}

pub fn config_pager_toml(command: &std::path::Path, args: &[&str]) -> String {
    config_table_toml("pager", command, args)
}

pub enum PagerChoice {
    CaptureScript,
    MissingBinary,
    Unset,
}

pub fn resolve_pager(choice: PagerChoice, env: &TestEnv) -> (Option<String>, Option<PathBuf>) {
    match choice {
        PagerChoice::CaptureScript => {
            let (script, output) = capture_script(env);
            (Some(script.to_str().unwrap().to_owned()), Some(output))
        }
        PagerChoice::MissingBinary => (
            Some(env.path("no-such-pager").to_string_lossy().into_owned()),
            None,
        ),
        PagerChoice::Unset => (None, None),
    }
}
