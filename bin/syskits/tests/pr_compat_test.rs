use std::process::{Command, Stdio};

fn run_pr_with_non_utf8_input(args: &[&str]) -> std::process::Output {
    let dir = tempfile::tempdir().expect("create temp dir");
    let input = dir.path().join("input");
    std::fs::write(&input, [b'a', 0x07, 0x80, b'\n']).expect("write non-utf8 input");

    let mut full_args = vec!["pr"];
    full_args.extend_from_slice(args);
    full_args.push(input.to_str().expect("input path is utf-8"));

    Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("TZ", "UTC0")
        .args(full_args)
        .output()
        .expect("run syskits pr")
}

#[test]
fn pr_pages_zero_is_rejected_without_reading_stdin() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("TZ", "UTC0")
        .args(["pr", "--pages=0"])
        .stdin(Stdio::piped())
        .output()
        .expect("run syskits pr --pages=0");

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
        "pr: invalid page range '0'\n"
    );
}

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

#[test]
fn pr_show_nonprinting_accepts_non_utf8_input() {
    let output = run_pr_with_non_utf8_input(&["-t", "-v"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"a\\007\\200\n");
    assert_eq!(output.stderr, b"");
}

#[test]
fn pr_without_show_nonprinting_preserves_non_utf8_bytes() {
    let output = run_pr_with_non_utf8_input(&["-t"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout hex: {:02x?}, stderr: {}",
        output.stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, [b'a', 0x07, 0x80, b'\n']);
    assert_eq!(output.stderr, b"");
}
