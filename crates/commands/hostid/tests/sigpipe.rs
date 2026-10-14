#![cfg(target_os = "linux")]

use std::process::{Command, Output};

fn run_hostid(binary: &str, ignore_sigpipe: bool) -> Output {
    const SCRIPT: &str = r#"
set -o pipefail
if test "$2" = ignored; then
    trap '' PIPE
fi
bash -c 'exec -a hostid "$0"' "$1" | true
status=("${PIPESTATUS[@]}")
printf '%s' "${status[0]}"
"#;

    Command::new("bash")
        .args([
            "-c",
            SCRIPT,
            "bash",
            binary,
            if ignore_sigpipe { "ignored" } else { "default" },
        ])
        .env("LC_ALL", "C")
        .output()
        .expect("run hostid with a closed pipe")
}

#[test]
fn closed_pipe_behavior_matches_gnu() {
    for ignore_sigpipe in [false, true] {
        let gnu = run_hostid("/usr/bin/hostid", ignore_sigpipe);
        let syskits = run_hostid(env!("CARGO_BIN_EXE_hostid"), ignore_sigpipe);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{ignore_sigpipe}");
        assert_eq!(syskits.stdout, gnu.stdout, "{ignore_sigpipe}");
        assert_eq!(syskits.stderr, gnu.stderr, "{ignore_sigpipe}");
    }
}
