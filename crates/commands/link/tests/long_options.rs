#[cfg(unix)]
#[test]
fn invalid_long_options_use_gnu_diagnostics() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let cases = [
        ("--h=value", "option '--help' doesn't allow an argument"),
        ("--v=value", "option '--version' doesn't allow an argument"),
        (
            "--=value",
            "option '--=value' is ambiguous; possibilities: '--help' '--version'",
        ),
        ("--helpful", "unrecognized option '--helpful'"),
        ("--unknown=value", "unrecognized option '--unknown=value'"),
    ];

    for (argument, message) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_link"))
            .arg0("link")
            .env("LC_ALL", "C")
            .arg(argument)
            .output()
            .expect("run link");

        assert_eq!(output.status.code(), Some(1), "{argument}");
        assert!(output.stdout.is_empty(), "{argument}");
        assert_eq!(
            output.stderr,
            format!("link: {message}\nTry 'link --help' for more information.\n").as_bytes(),
            "{argument}"
        );
    }
}
