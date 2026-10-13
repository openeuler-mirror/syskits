#[cfg(unix)]
#[test]
fn regular_expression_offsets_accept_leading_ascii_whitespace() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp_dir.path().join("input"), b"a\nb\nc\n").expect("write input");

    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .env("LC_ALL", "C")
        .current_dir(temp_dir.path())
        .args(["input", "/a/ 1"])
        .output()
        .expect("run csplit with a whitespace-prefixed offset");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"2\n4\n");
    assert!(output.stderr.is_empty());
    assert_eq!(std::fs::read(temp_dir.path().join("xx00")).unwrap(), b"a\n");
    assert_eq!(
        std::fs::read(temp_dir.path().join("xx01")).unwrap(),
        b"b\nc\n"
    );
}
