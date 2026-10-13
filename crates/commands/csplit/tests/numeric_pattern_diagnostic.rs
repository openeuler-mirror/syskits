#[cfg(target_os = "linux")]
#[test]
fn numeric_pattern_diagnostics_preserve_the_original_operand() {
    use std::process::Command;

    for (arguments, expected_status, expected_stderr) in [
        (
            vec!["-q", "input", "2", "+2"],
            0,
            b"csplit: warning: line number '+2' is the same as preceding line number\n".as_slice(),
        ),
        (
            vec!["-q", "input", "2", " \t+2"],
            0,
            b"csplit: warning: line number ' \\t+2' is the same as preceding line number\n"
                .as_slice(),
        ),
        (
            vec!["input", "2", "+1"],
            1,
            b"csplit: line number '+1' is smaller than preceding line number, 2\n".as_slice(),
        ),
    ] {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        std::fs::write(temp_dir.path().join("input"), b"a\nb\nc\n").expect("write input");

        let output = Command::new("bash")
            .args(["-c", "exec -a csplit \"$0\" \"$@\""])
            .arg(env!("CARGO_BIN_EXE_csplit"))
            .args(arguments)
            .env("LC_ALL", "C")
            .current_dir(temp_dir.path())
            .output()
            .expect("run csplit");

        assert_eq!(output.status.code(), Some(expected_status), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert_eq!(output.stderr, expected_stderr, "{output:?}");
    }
}
