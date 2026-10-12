#[cfg(unix)]
#[test]
fn line_number_accepts_leading_ascii_whitespace() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp_dir.path().join("input"), b"a\n").expect("write input");

    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .current_dir(temp_dir.path())
        .arg("input")
        .arg(" \t+1")
        .output()
        .expect("run csplit");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"0\n2\n");
    assert!(output.stderr.is_empty());
    assert_eq!(
        std::fs::read(temp_dir.path().join("xx01")).expect("read xx01"),
        b"a\n"
    );
}
