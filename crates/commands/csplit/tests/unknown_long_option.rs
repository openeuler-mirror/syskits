#[cfg(unix)]
#[test]
fn unknown_long_options_use_gnu_diagnostics() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for option in ["--unknown", "--unknown=value"] {
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
                "csplit: unrecognized option '{option}'\nTry 'csplit --help' for more information.\n"
            )
            .as_bytes(),
            "{option}"
        );
    }
}
