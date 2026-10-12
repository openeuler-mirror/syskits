#[cfg(unix)]
#[test]
fn repeated_options_follow_gnu_override_rules() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for arguments in [
        vec!["-q", "-q"],
        vec!["-k", "-k"],
        vec!["--suppress-matched", "--suppress-matched"],
        vec!["--digits=2", "--digits=3"],
        vec!["--prefix=first", "--prefix=second"],
        vec!["--suffix-format=%d", "--suffix-format=%x"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
            .arg0("csplit")
            .args(arguments)
            .output()
            .expect("run csplit with repeated option");

        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            output.stderr,
            b"csplit: missing operand\nTry 'csplit --help' for more information.\n"
        );
    }

    let temp_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp_dir.path().join("input"), b"a\n").expect("write input");

    let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .current_dir(temp_dir.path())
        .args([
            "--prefix=first",
            "--prefix=second",
            "--digits=1",
            "--digits=3",
            "input",
            "1",
        ])
        .output()
        .expect("run csplit with repeated values");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"0\n2\n");
    assert!(output.stderr.is_empty());
    assert!(!temp_dir.path().join("first000").exists());
    assert_eq!(
        std::fs::read(temp_dir.path().join("second000")).expect("read first split"),
        b""
    );
    assert_eq!(
        std::fs::read(temp_dir.path().join("second001")).expect("read second split"),
        b"a\n"
    );
}
