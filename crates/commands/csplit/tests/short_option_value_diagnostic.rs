#[cfg(unix)]
#[test]
fn short_options_missing_values_use_gnu_diagnostics() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    for (option, short) in [("-n", 'n'), ("-b", 'b'), ("-f", 'f'), ("-kn", 'n')] {
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
                "csplit: option requires an argument -- '{short}'\nTry 'csplit --help' for more information.\n"
            )
            .as_bytes(),
            "{option}"
        );
    }
}
