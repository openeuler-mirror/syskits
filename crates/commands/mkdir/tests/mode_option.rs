#[cfg(unix)]
#[test]
fn short_mode_option_requires_an_argument() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for argument in ["-m", "-pm"] {
        let output = Command::new(env!("CARGO_BIN_EXE_mkdir"))
            .arg0("mkdir")
            .env("LC_ALL", "C")
            .arg(argument)
            .output()
            .expect("run mkdir with a mode option missing its argument");

        assert_eq!(output.status.code(), Some(1), "{argument}");
        assert!(output.stdout.is_empty(), "{argument}");
        assert_eq!(
            output.stderr,
            b"mkdir: option requires an argument -- 'm'\nTry 'mkdir --help' for more information.\n",
            "{argument}"
        );
    }
}
