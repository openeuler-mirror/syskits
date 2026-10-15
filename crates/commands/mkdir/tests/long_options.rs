#[cfg(unix)]
#[test]
fn invalid_long_options_use_gnu_diagnostics() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    let cases = [
        ("--help=x", "option '--help' doesn't allow an argument"),
        (
            "--parents=x",
            "option '--parents' doesn't allow an argument",
        ),
        ("--m", "option '--mode' requires an argument"),
        (
            "--v",
            "option '--v' is ambiguous; possibilities: '--verbose' '--version'",
        ),
        (
            "--ver",
            "option '--ver' is ambiguous; possibilities: '--verbose' '--version'",
        ),
        ("--unknown", "unrecognized option '--unknown'"),
    ];

    for (argument, message) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_mkdir"))
            .arg0("mkdir")
            .env("LC_ALL", "C")
            .arg(argument)
            .output()
            .expect("run mkdir with an invalid long option");

        assert_eq!(output.status.code(), Some(1), "{argument}");
        assert!(output.stdout.is_empty(), "{argument}");
        assert_eq!(
            output.stderr,
            format!("mkdir: {message}\nTry 'mkdir --help' for more information.\n").as_bytes(),
            "{argument}"
        );
    }
}
