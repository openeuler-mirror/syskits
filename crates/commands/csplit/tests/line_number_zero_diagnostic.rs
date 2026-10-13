#[cfg(target_os = "linux")]
#[test]
fn zero_line_numbers_preserve_the_original_operand_in_diagnostics() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let temp_dir = tempfile::tempdir().expect("create temporary directory");
    std::fs::write(temp_dir.path().join("input"), b"a\n").expect("write input");

    for (operand, expected) in [
        (
            "+0",
            b"csplit: +0: line number must be greater than zero\n".as_slice(),
        ),
        (
            " \t0",
            b"csplit:  \t0: line number must be greater than zero\n".as_slice(),
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
            .arg0("csplit")
            .args(["input", operand])
            .current_dir(temp_dir.path())
            .output()
            .expect("run csplit");

        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert_eq!(output.stderr, expected, "{output:?}");
    }
}
