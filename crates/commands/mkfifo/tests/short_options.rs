#[cfg(unix)]
#[test]
fn invalid_short_options_use_gnu_diagnostics() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for (arguments, message) in [
        (vec!["-h"], "invalid option -- 'h'"),
        (vec!["-V"], "invalid option -- 'V'"),
        (vec!["-x"], "invalid option -- 'x'"),
        (vec!["-Zm"], "option requires an argument -- 'm'"),
        (vec!["-m"], "option requires an argument -- 'm'"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_mkfifo"))
            .arg0("mkfifo")
            .env("LC_ALL", "C")
            .args(arguments)
            .output()
            .expect("run mkfifo with an invalid short option");

        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            output.stderr,
            format!("mkfifo: {message}\nTry 'mkfifo --help' for more information.\n").as_bytes()
        );
    }
}
