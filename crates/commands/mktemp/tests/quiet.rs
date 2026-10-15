#[cfg(unix)]
#[test]
fn quiet_suppresses_stdout_write_error() {
    use std::fs::File;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let output = Command::new(env!("CARGO_BIN_EXE_mktemp"))
        .arg0("mktemp")
        .args(["-q", "/tmp/mktemp-quiet-write.XXXXXX"])
        .stdout(Stdio::from(
            File::options().write(true).open("/dev/full").unwrap(),
        ))
        .stderr(Stdio::piped())
        .output()
        .expect("run mktemp with unwritable stdout");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
}

#[cfg(unix)]
#[test]
fn stdout_write_error_uses_gnu_diagnostic_context() {
    use std::fs::File;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let output = Command::new(env!("CARGO_BIN_EXE_mktemp"))
        .arg0("mktemp")
        .env("LC_ALL", "C")
        .arg("/tmp/mktemp-write-error.XXXXXX")
        .stdout(Stdio::from(
            File::options().write(true).open("/dev/full").unwrap(),
        ))
        .stderr(Stdio::piped())
        .output()
        .expect("run mktemp with unwritable stdout");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "mktemp: write error: No space left on device\n"
    );
}
