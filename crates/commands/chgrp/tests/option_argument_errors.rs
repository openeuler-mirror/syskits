#[cfg(unix)]
#[test]
fn chgrp_reports_long_option_argument_errors_like_gnu() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for (args, message) in [
        (
            vec!["--reference"],
            "option '--reference' requires an argument",
        ),
        (vec!["--ref"], "option '--reference' requires an argument"),
        (
            vec!["--changes=yes", "0", "/tmp"],
            "option '--changes' doesn't allow an argument",
        ),
        (
            vec!["--silent=yes", "0", "/tmp"],
            "option '--silent' doesn't allow an argument",
        ),
        (
            vec!["--quiet=yes", "0", "/tmp"],
            "option '--quiet' doesn't allow an argument",
        ),
        (
            vec!["--verbose=yes", "0", "/tmp"],
            "option '--verbose' doesn't allow an argument",
        ),
        (
            vec!["--dereference=yes", "0", "/tmp"],
            "option '--dereference' doesn't allow an argument",
        ),
        (
            vec!["--no-dereference=yes", "0", "/tmp"],
            "option '--no-dereference' doesn't allow an argument",
        ),
        (
            vec!["--preserve-root=yes", "0", "/tmp"],
            "option '--preserve-root' doesn't allow an argument",
        ),
        (
            vec!["--no-preserve-root=yes", "0", "/tmp"],
            "option '--no-preserve-root' doesn't allow an argument",
        ),
        (
            vec!["--recursive=yes", "0", "/tmp"],
            "option '--recursive' doesn't allow an argument",
        ),
        (
            vec!["--help=yes"],
            "option '--help' doesn't allow an argument",
        ),
        (
            vec!["--version=yes"],
            "option '--version' doesn't allow an argument",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
            .env("LC_ALL", "C")
            .arg0("chgrp")
            .args(args)
            .output()
            .expect("run chgrp");

        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            format!("chgrp: {message}\nTry 'chgrp --help' for more information.\n")
        );
    }
}
