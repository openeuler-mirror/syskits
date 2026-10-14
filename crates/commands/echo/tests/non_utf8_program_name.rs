#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn help_preserves_non_utf8_program_name_bytes() {
    let program_name = OsString::from_vec(b"ec\xffho".to_vec());
    let output = Command::new(env!("CARGO_BIN_EXE_echo"))
        .arg0(program_name)
        .arg("--help")
        .env("LC_ALL", "C")
        .output()
        .expect("run echo --help");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert!(output.stdout.starts_with(
        b"Usage: ec\xffho [SHORT-OPTION]... [STRING]...\n  or:  ec\xffho LONG-OPTION\n"
    ));
}
