#[cfg(unix)]
#[test]
fn suffix_validation_runs_after_operand_checks() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let cases = [
        (
            vec!["--suffix-format=literal"],
            "missing operand".to_string(),
            true,
        ),
        (
            vec!["--suffix-format=literal", "input"],
            format!(
                "missing operand after {}",
                ctcore::ct_display::locale_quote("input")
            ),
            true,
        ),
        (
            vec!["--digits=bad", "--suffix-format=literal"],
            format!(
                "invalid number: {}",
                ctcore::ct_display::locale_quote("bad")
            ),
            false,
        ),
    ];

    for (arguments, message, usage_error) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
            .arg0("csplit")
            .args(arguments)
            .output()
            .expect("run csplit");

        assert_eq!(output.status.code(), Some(1), "{message}");
        assert!(output.stdout.is_empty(), "{message}");
        let usage = usage_error
            .then_some("Try 'csplit --help' for more information.\n")
            .unwrap_or_default();
        assert_eq!(
            output.stderr,
            format!("csplit: {message}\n{usage}").as_bytes()
        );
    }
}
