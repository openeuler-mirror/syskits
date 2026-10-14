#![cfg(target_os = "linux")]

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};

fn run_hostid(binary: &str, args: &[OsString]) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("hostid")
        .args(args)
        .stdin(Stdio::null())
        .env("LC_ALL", "C")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run hostid")
}

#[test]
fn non_utf8_option_diagnostics_match_gnu() {
    let cases = [
        vec![OsString::from_vec(b"-\xff".to_vec())],
        vec![OsString::from_vec(b"-a\xff".to_vec())],
        vec![OsString::from_vec(b"--\xff".to_vec())],
        vec![OsString::from_vec(b"--a\xff".to_vec())],
        vec![OsString::from_vec(b"--\xff=value".to_vec())],
    ];

    for args in cases {
        let gnu = run_hostid("/usr/bin/hostid", &args);
        let syskits = run_hostid(env!("CARGO_BIN_EXE_hostid"), &args);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{args:?}");
        assert_eq!(syskits.stdout, gnu.stdout, "{args:?}");
        assert_eq!(syskits.stderr, gnu.stderr, "{args:?}");
    }
}
