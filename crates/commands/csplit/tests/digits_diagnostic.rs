#[cfg(unix)]
#[test]
fn digits_match_gnu_range_diagnostics_and_option_precedence() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let cases = [
        (vec!["--digits=bad"], "bad", None),
        (
            vec!["-n", "-1"],
            "-1",
            Some("Numerical result out of range"),
        ),
        (
            vec!["--digits=2147483648"],
            "2147483648",
            Some("Value too large for defined data type"),
        ),
        (
            vec!["--digits=999999999999999999999999"],
            "999999999999999999999999",
            Some("Value too large for defined data type"),
        ),
    ];

    for (arguments, value, reason) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
            .arg0("csplit")
            .args(arguments)
            .output()
            .expect("run csplit");

        let suffix = reason.map_or_else(String::new, |reason| format!(": {reason}"));
        assert_eq!(output.status.code(), Some(1), "{value}");
        assert!(output.stdout.is_empty(), "{value}");
        assert_eq!(
            output.stderr,
            format!(
                "csplit: invalid number: {}{suffix}\n",
                ctcore::ct_display::locale_quote(value)
            )
            .as_bytes(),
            "{value}"
        );
    }
}
