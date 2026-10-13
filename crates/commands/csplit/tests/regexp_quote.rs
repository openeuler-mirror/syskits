#[cfg(unix)]
#[test]
fn regular_expression_diagnostics_use_gnu_locale_quoting() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp_dir.path().join("input"), b"a\naa\naaa\n").expect("write input");

    let cases = [
        (
            "/a\\{2,1\\}/",
            b"".as_slice(),
            b"csplit: '/a\\\\{2,1\\\\}/': invalid regular expression: Invalid content of \\{\\}\n"
                .as_slice(),
        ),
        (
            "/\\{2,1\\}/",
            b"9\n".as_slice(),
            b"csplit: '/\\\\{2,1\\\\}/': match not found\n".as_slice(),
        ),
    ];

    for (pattern, expected_stdout, expected_stderr) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
            .arg0("csplit")
            .env("LC_ALL", "C")
            .current_dir(temp_dir.path())
            .args(["input", pattern])
            .output()
            .expect("run csplit with quoted BRE diagnostic");

        assert_eq!(output.status.code(), Some(1), "{pattern}");
        assert_eq!(output.stdout, expected_stdout, "{pattern}");
        assert_eq!(output.stderr, expected_stderr, "{pattern}");
    }
}
