#[cfg(unix)]
#[test]
fn missing_regexp_delimiter_has_a_specific_diagnostic() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp_dir.path().join("input"), b"a\n").expect("write input");

    for (pattern, expected) in [
        ("/abc", "csplit: /abc: closing delimiter '/' missing\n"),
        ("%abc", "csplit: %abc: closing delimiter '%' missing\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
            .arg0("csplit")
            .current_dir(temp_dir.path())
            .arg("input")
            .arg(pattern)
            .output()
            .expect("run csplit");

        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, expected.as_bytes());
    }
}
