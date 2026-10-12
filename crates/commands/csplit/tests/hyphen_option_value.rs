#[cfg(unix)]
#[test]
fn prefix_and_suffix_options_accept_hyphen_prefixed_values() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp_dir.path().join("input"), b"a\n").expect("write input");

    let prefix = Command::new(env!("CARGO_BIN_EXE_csplit"))
        .arg0("csplit")
        .current_dir(temp_dir.path())
        .args(["-f", "-prefix", "input", "1"])
        .output()
        .expect("run csplit with prefix");
    assert_eq!(prefix.status.code(), Some(0));
    assert_eq!(prefix.stdout, b"0\n2\n");
    assert!(prefix.stderr.is_empty());
    assert_eq!(
        std::fs::read(temp_dir.path().join("-prefix00")).expect("read first split"),
        b""
    );
    assert_eq!(
        std::fs::read(temp_dir.path().join("-prefix01")).expect("read second split"),
        b"a\n"
    );

    for arguments in [
        vec!["-b", "-h", "input", "1"],
        vec!["--suffix-format", "-h", "input", "1"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
            .arg0("csplit")
            .current_dir(temp_dir.path())
            .args(arguments)
            .output()
            .expect("run csplit with suffix format");

        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            output.stderr,
            b"csplit: missing % conversion specification in suffix\n"
        );
    }
}
