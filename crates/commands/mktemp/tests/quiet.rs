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
