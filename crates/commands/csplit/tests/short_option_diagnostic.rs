#[cfg(unix)]
#[test]
fn short_help_and_version_options_match_gnu_errors() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for option in ["-h", "-V", "-kh", "-qV"] {
        let output = Command::new(env!("CARGO_BIN_EXE_csplit"))
            .arg0("csplit")
            .arg(option)
            .output()
            .expect("run csplit");

        let invalid = option
            .chars()
            .find(|character| matches!(character, 'h' | 'V'))
            .expect("invalid short option");
        assert_eq!(output.status.code(), Some(1), "{option}");
        assert!(output.stdout.is_empty(), "{option}");
        assert_eq!(
            output.stderr,
            format!(
                "csplit: invalid option -- '{invalid}'\nTry 'csplit --help' for more information.\n"
            )
            .as_bytes(),
            "{option}"
        );
    }
}
