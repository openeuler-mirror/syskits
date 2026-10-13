#[cfg(unix)]
#[test]
fn line_numbers_larger_than_intmax_are_invalid_patterns() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp_dir.path().join("input"), b"a\nb\nc\n").expect("write input");

    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .env("LC_ALL", "C")
        .current_dir(temp_dir.path())
        .args(["input", "9223372036854775808"])
        .output()
        .expect("run csplit with an out-of-range line number");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"csplit: '9223372036854775808': invalid pattern\n"
    );
    assert!(!temp_dir.path().join("xx00").exists());
}
