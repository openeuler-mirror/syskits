#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn non_utf8_arguments_are_written_without_lossy_conversion() {
    let argument = OsString::from_vec(vec![b'a', 0xff, b'\\', b'n', 0x80]);

    let output = Command::new(env!("CARGO_BIN_EXE_echo"))
        .arg0("echo")
        .args(["-e"])
        .arg(argument)
        .env("LC_ALL", "C")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, b"a\xff\n\x80\n");
}
