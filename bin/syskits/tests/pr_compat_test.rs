use std::process::{Command, Stdio};

#[test]
fn pr_number_lines_invalid_argument_exits_without_reading_stdin() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C.UTF-8")
        .args(["pr", "--number-lines=invalid"])
        .stdin(Stdio::null())
        .output()
        .expect("run syskits pr --number-lines=invalid");

    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "pr: '-n' extra characters or invalid number in the argument: ‘nvalid’\nTry 'pr --help' for more information.\n"
    );
}
