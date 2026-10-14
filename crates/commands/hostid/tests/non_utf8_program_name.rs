#![cfg(target_os = "linux")]

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};

fn run_hostid(binary: &str, program_name: &OsStr, args: &[OsString]) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0(program_name)
        .args(args)
        .stdin(Stdio::null())
        .env("LC_ALL", "C")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run hostid")
}

#[test]
fn non_utf8_program_name_diagnostics_match_gnu() {
    let program_names = [
        OsString::from_vec(b"h\xff".to_vec()),
        OsString::from_vec(b"/tmp/h\xff".to_vec()),
    ];
    let cases = [
        vec![OsString::from("value")],
        vec![OsString::from("-h")],
        vec![OsString::from("--invalid")],
        vec![OsString::from_vec(b"-\xff".to_vec())],
        vec![OsString::from_vec(b"--\xff".to_vec())],
    ];

    for program_name in program_names {
        for args in &cases {
            let gnu = run_hostid("/usr/bin/hostid", &program_name, args);
            let syskits = run_hostid(env!("CARGO_BIN_EXE_hostid"), &program_name, args);

            assert_eq!(
                syskits.status.code(),
                gnu.status.code(),
                "{program_name:?}: {args:?}"
            );
            assert_eq!(syskits.stdout, gnu.stdout, "{program_name:?}: {args:?}");
            assert_eq!(syskits.stderr, gnu.stderr, "{program_name:?}: {args:?}");
        }
    }
}
