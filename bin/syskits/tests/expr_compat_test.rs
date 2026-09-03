use std::process::Command;

#[test]
fn expr_help_and_version_with_extra_operand_are_syntax_errors() {
    for option in ["--help", "--version"] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .env("LC_ALL", "C.UTF-8")
            .args(["expr", option, "/no/such/path"])
            .output()
            .expect("run syskits expr");

        assert_eq!(
            output.status.code(),
            Some(2),
            "{option} status, stdout: {}, stderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stdout.is_empty(),
            "{option} stdout: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("expr: syntax error: unexpected argument ‘/no/such/path’\n"),
            "{option} stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn expr_syntax_error_quotes_follow_c_locale() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .args(["expr", "--version", "/no/such/path"])
        .output()
        .expect("run syskits expr");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("expr: syntax error: unexpected argument '/no/such/path'\n"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
