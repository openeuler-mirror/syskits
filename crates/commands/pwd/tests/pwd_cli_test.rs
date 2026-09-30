use std::process::Command;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

#[test]
fn non_option_arguments_emit_gnu_warning() {
    let output = Command::new(env!("CARGO_BIN_EXE_pwd"))
        .arg0("pwd")
        .arg("ignored")
        .env("LC_ALL", "C")
        .output()
        .expect("run pwd");

    assert_eq!(output.status.code(), Some(0));
    assert!(!output.stdout.is_empty());
    assert_eq!(output.stderr, b"pwd: ignoring non-option arguments\n");
}
