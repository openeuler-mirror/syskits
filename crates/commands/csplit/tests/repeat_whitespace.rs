#[cfg(unix)]
#[test]
fn repeat_counts_accept_leading_ascii_whitespace() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp_dir.path().join("input"), b"a\nb\nc\n").expect("write input");

    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .env("LC_ALL", "C")
        .current_dir(temp_dir.path())
        .args(["input", "/a/", "{ 1}"])
        .output()
        .expect("run csplit with a whitespace-prefixed repeat count");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"0\n6\n");
    assert_eq!(
        output.stderr,
        b"csplit: '/a/': match not found on repetition 1\n"
    );
    assert!(!temp_dir.path().join("xx00").exists());
}
