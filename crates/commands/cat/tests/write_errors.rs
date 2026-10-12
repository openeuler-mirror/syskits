#[cfg(target_os = "linux")]
#[test]
fn cat_reports_stdout_write_errors() {
    use std::fs::OpenOptions;
    use std::process::{Command, Stdio};

    let temp_dir = tempfile::tempdir().expect("tempdir");
    let input = temp_dir.path().join("input");
    let program_name = env!("CARGO_BIN_EXE_cat");
    std::fs::write(&input, b"a\tb").expect("write input");

    for option in [None, Some("-T")] {
        let stdout = OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .expect("open /dev/full");
        let mut command = Command::new(program_name);
        if let Some(option) = option {
            command.arg(option);
        }
        command.arg(&input).stdout(Stdio::from(stdout));

        let output = command.output().expect("run cat");
        assert_eq!(output.status.code(), Some(1), "option: {option:?}");
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr).expect("stderr is UTF-8"),
            format!("{program_name}: write error: No space left on device\n"),
            "option: {option:?}"
        );
    }
}
