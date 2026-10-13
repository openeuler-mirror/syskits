#[cfg(target_os = "linux")]
#[test]
fn closed_stdout_pipe_terminates_with_sigpipe() {
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    let output = Command::new("bash")
        .arg("-c")
        .arg(
            "set -o pipefail; seq 10000 | \"$1\" - '/^/' '{*}' | head -n 1 >/dev/null; printf '%s' \"${PIPESTATUS[1]}\"",
        )
        .arg("bash")
        .arg(env!("CARGO_BIN_EXE_csplit"))
        .env("LC_ALL", "C")
        .current_dir(temp_dir.path())
        .output()
        .expect("run csplit with a closed stdout pipe");

    assert!(output.status.success());
    assert_eq!(output.stdout, b"141");
    assert!(output.stderr.is_empty());
}
