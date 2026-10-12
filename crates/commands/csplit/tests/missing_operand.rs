#[cfg(unix)]
#[test]
fn missing_operands_use_gnu_diagnostics() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let cases = [
        (Vec::<&str>::new(), "missing operand".to_string()),
        (vec!["-q"], "missing operand".to_string()),
        (
            vec!["input"],
            format!(
                "missing operand after {}",
                ctcore::ct_display::locale_quote("input")
            ),
        ),
        (
            vec!["--", "/abc/"],
            format!(
                "missing operand after {}",
                ctcore::ct_display::locale_quote("/abc/")
            ),
        ),
    ];

    for (arguments, message) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
            .arg0("csplit")
            .args(arguments)
            .output()
            .expect("run csplit");

        assert_eq!(output.status.code(), Some(1), "{message}");
        assert!(output.stdout.is_empty(), "{message}");
        assert_eq!(
            output.stderr,
            format!("csplit: {message}\nTry 'csplit --help' for more information.\n").as_bytes(),
            "{message}"
        );
    }
}
