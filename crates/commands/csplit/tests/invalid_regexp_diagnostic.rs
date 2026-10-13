#[cfg(unix)]
#[test]
fn invalid_bre_diagnostics_match_gnu_error_categories() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp_dir.path().join("input"), b"aaa\n").expect("write input");

    let cases = [
        ("/[/", "Invalid regular expression"),
        ("/a\\{x\\}/", "Invalid content of \\{\\}"),
        ("/a\\{2/", "Unmatched \\{"),
        ("/\\(a/", "Unmatched ( or \\("),
        ("/a\\)b/", "Unmatched ) or \\)"),
        ("/[[:unknown:]]/", "Invalid character class name"),
    ];

    for (pattern, reason) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
            .arg0("csplit")
            .env("LC_ALL", "C")
            .current_dir(temp_dir.path())
            .args(["input", pattern])
            .output()
            .expect("run csplit with invalid BRE");

        assert_eq!(output.status.code(), Some(1), "{pattern}");
        assert!(output.stdout.is_empty(), "{pattern}");
        assert_eq!(
            output.stderr,
            format!(
                "csplit: '{}': invalid regular expression: {reason}\n",
                pattern.replace('\\', "\\\\")
            )
            .as_bytes(),
            "{pattern}"
        );
    }
}
