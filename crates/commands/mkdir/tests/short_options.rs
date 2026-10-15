#[cfg(unix)]
#[test]
fn invalid_short_options_use_gnu_diagnostics() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for (arguments, option) in [
        (vec!["-h"], 'h'),
        (vec!["-V"], 'V'),
        (vec!["-w"], 'w'),
        (vec!["-hp"], 'h'),
        (vec!["-pH"], 'H'),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_mkdir"))
            .arg0("mkdir")
            .env("LC_ALL", "C")
            .args(arguments)
            .output()
            .expect("run mkdir with an invalid short option");

        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            output.stderr,
            format!(
                "mkdir: invalid option -- '{option}'\nTry 'mkdir --help' for more information.\n"
            )
            .as_bytes()
        );
    }
}
