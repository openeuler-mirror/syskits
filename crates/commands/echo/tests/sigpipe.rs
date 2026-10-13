#![cfg(target_os = "linux")]

use std::process::Command;

const CLOSED_PIPE_SCRIPT: &str = r#"
payload=$(head -c 65536 /dev/zero | tr '\0' x)
set -o pipefail
"$1" "$payload" | head -n 0 >/dev/null
printf '%s' "${PIPESTATUS[0]}"
"#;

#[test]
fn closed_stdout_pipe_terminates_with_sigpipe() {
    let output = Command::new("bash")
        .args(["-c", CLOSED_PIPE_SCRIPT, "bash", env!("CARGO_BIN_EXE_echo")])
        .env("LC_ALL", "C")
        .output()
        .expect("run echo with a closed stdout pipe");

    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"141");
    assert!(output.stderr.is_empty());
}

#[test]
fn inherited_ignored_sigpipe_reports_a_write_error() {
    const SCRIPT: &str = r#"
trap '' PIPE
payload=$(head -c 65536 /dev/zero | tr '\0' x)
set -o pipefail
bash -c 'exec -a echo "$0" "$1"' "$1" "$payload" 2>stderr | head -n 0 >/dev/null
printf '%s' "${PIPESTATUS[0]}" > status
"#;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    let output = Command::new("bash")
        .args(["-c", SCRIPT, "bash", env!("CARGO_BIN_EXE_echo")])
        .env("LC_ALL", "C")
        .current_dir(temp_dir.path())
        .output()
        .expect("run echo with inherited ignored SIGPIPE");

    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(std::fs::read(temp_dir.path().join("status")).unwrap(), b"1");
    assert_eq!(
        std::fs::read(temp_dir.path().join("stderr")).unwrap(),
        b"echo: write error: Broken pipe\n"
    );
}
