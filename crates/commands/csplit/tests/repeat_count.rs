#[cfg(unix)]
#[test]
fn invalid_repeat_counts_have_gnu_diagnostics_without_panicking() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp_dir.path().join("input"), b"a\n").expect("write input");

    let cases = [
        ("{2", "csplit: '{2': '}' is required in repeat count\n"),
        ("{}", "csplit: '{'}: integer required between '{' and '}'\n"),
        (
            "{x}",
            "csplit: '{x'}: integer required between '{' and '}'\n",
        ),
        (
            "{9223372036854775808}",
            "csplit: '{9223372036854775808'}: integer required between '{' and '}'\n",
        ),
    ];

    for (repeat_count, expected) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
            .arg0("csplit")
            .env("LC_ALL", "C")
            .current_dir(temp_dir.path())
            .args(["input", "1", repeat_count])
            .output()
            .expect("run csplit");

        assert_eq!(output.status.code(), Some(1), "{repeat_count}");
        assert!(output.stdout.is_empty(), "{repeat_count}");
        assert_eq!(output.stderr, expected.as_bytes(), "{repeat_count}");
    }
}
