#[cfg(unix)]
#[test]
fn ambiguous_long_options_list_gnu_candidates() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let cases = [
        ("--s", "'--silent' '--suffix-format' '--suppress-matched'"),
        ("--su", "'--suffix-format' '--suppress-matched'"),
        ("--s=x", "'--silent' '--suffix-format' '--suppress-matched'"),
        (
            "--=x",
            "'--digits' '--quiet' '--silent' '--keep-files' '--elide-empty-files' '--prefix' '--suffix-format' '--suppress-matched' '--help' '--version'",
        ),
    ];

    for (option, possibilities) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
            .arg0("csplit")
            .arg(option)
            .output()
            .expect("run csplit");

        assert_eq!(output.status.code(), Some(1), "{option}");
        assert!(output.stdout.is_empty(), "{option}");
        assert_eq!(
            output.stderr,
            format!(
                "csplit: option '{option}' is ambiguous; possibilities: {possibilities}\nTry 'csplit --help' for more information.\n"
            )
            .as_bytes(),
            "{option}"
        );
    }
}
