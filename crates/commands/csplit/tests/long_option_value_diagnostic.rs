#[cfg(unix)]
#[test]
fn long_option_value_errors_use_gnu_diagnostics() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let cases = [
        ("--d", "option '--digits' requires an argument"),
        ("--suf", "option '--suffix-format' requires an argument"),
        ("--help=x", "option '--help' doesn't allow an argument"),
        (
            "--suppress-matched=x",
            "option '--suppress-matched' doesn't allow an argument",
        ),
    ];

    for (option, message) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
            .arg0("csplit")
            .arg(option)
            .output()
            .expect("run csplit");

        assert_eq!(output.status.code(), Some(1), "{option}");
        assert!(output.stdout.is_empty(), "{option}");
        assert_eq!(
            output.stderr,
            format!("csplit: {message}\nTry 'csplit --help' for more information.\n").as_bytes(),
            "{option}"
        );
    }
}
