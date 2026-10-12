#[cfg(unix)]
#[test]
fn empty_regular_expression_splits_before_the_first_line() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    let input = temp_dir.path().join("input");
    std::fs::write(&input, b"a\nb\n").expect("write input");

    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .current_dir(temp_dir.path())
        .arg("input")
        .arg("//")
        .output()
        .expect("run csplit");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"0\n4\n");
    assert!(output.stderr.is_empty());
    assert_eq!(
        std::fs::read(temp_dir.path().join("xx00")).expect("read xx00"),
        b""
    );
    assert_eq!(
        std::fs::read(temp_dir.path().join("xx01")).expect("read xx01"),
        b"a\nb\n"
    );
}
