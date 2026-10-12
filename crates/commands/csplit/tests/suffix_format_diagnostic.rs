#[cfg(unix)]
#[test]
fn suffix_format_validation_uses_gnu_diagnostics() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let cases = [
        (
            "literal",
            "csplit: missing % conversion specification in suffix\n",
        ),
        ("%", "csplit: missing conversion specifier in suffix\n"),
        ("%s", "csplit: invalid conversion specifier in suffix: s\n"),
        (
            "%llx",
            "csplit: invalid conversion specifier in suffix: l\n",
        ),
        (
            "%#d",
            "csplit: invalid flags in conversion specification: %#d\n",
        ),
        (
            "%'x",
            "csplit: invalid flags in conversion specification: %'x\n",
        ),
        (
            "%d-%x",
            "csplit: too many % conversion specifications in suffix\n",
        ),
    ];

    for (suffix, expected) in cases {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp_dir.path().join("input"), b"a\n").expect("write input");
        let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
            .arg0("csplit")
            .current_dir(temp_dir.path())
            .args(["-b", suffix, "input", "1"])
            .output()
            .expect("run csplit");

        assert_eq!(output.status.code(), Some(1), "{suffix}");
        assert!(output.stdout.is_empty(), "{suffix}");
        assert_eq!(output.stderr, expected.as_bytes(), "{suffix}");
    }
}
