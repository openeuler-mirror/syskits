#![cfg(target_os = "linux")]

use std::process::Command;

#[test]
fn stdout_closed_before_start_reports_bad_file_descriptor() {
    const SCRIPT: &str = r#"
exec 3>&1
exec 1>&-
bash -c 'exec -a echo "$0" value' "$1"
printf '%s' "$?" >&3
"#;

    let output = Command::new("bash")
        .args(["-c", SCRIPT, "bash", env!("CARGO_BIN_EXE_echo")])
        .env("LC_ALL", "C")
        .output()
        .expect("run echo with stdout closed before startup");

    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, b"1");
    assert_eq!(output.stderr, b"echo: write error: Bad file descriptor\n");
}
