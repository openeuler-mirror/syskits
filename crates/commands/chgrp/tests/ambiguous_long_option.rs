#[cfg(unix)]
#[test]
fn chgrp_reports_ambiguous_long_options_like_gnu() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for (argument, possibilities) in [
        ("--ver", "'--verbose' '--version'"),
        ("--re", "'--recursive' '--reference'"),
        ("--no", "'--no-dereference' '--no-preserve-root'"),
        ("--ver=1", "'--verbose' '--version'"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_chgrp"))
            .env("LC_ALL", "C")
            .arg0("chgrp")
            .arg(argument)
            .output()
            .expect("run chgrp");

        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            format!(
                "chgrp: option '{argument}' is ambiguous; possibilities: {possibilities}\nTry 'chgrp --help' for more information.\n"
            )
        );
    }
}
