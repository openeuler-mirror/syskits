#[cfg(unix)]
#[test]
fn help_and_version_report_stdout_write_errors() {
    use std::fs::OpenOptions;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    for option in ["--help", "--version"] {
        let output = Command::new(env!("CARGO_BIN_EXE_link"))
            .arg0("link")
            .env("LC_ALL", "C")
            .arg(option)
            .stdout(Stdio::from(
                OpenOptions::new()
                    .write(true)
                    .open("/dev/full")
                    .expect("open /dev/full"),
            ))
            .output()
            .expect("run link");

        assert_eq!(output.status.code(), Some(1), "{option}");
        assert_eq!(
            output.stderr, b"link: write error: No space left on device\n",
            "{option}"
        );
    }
}
