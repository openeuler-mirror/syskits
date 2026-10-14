#![cfg(target_os = "linux")]

use std::ffi::{OsStr, OsString};
use std::fs::OpenOptions;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};

fn run_hostid(binary: &str, locale: &str, program_name: &OsStr) -> Output {
    let stdout = OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full");
    let mut command = Command::new(binary);
    command
        .arg0(program_name)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .env("LC_ALL", locale)
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run hostid with /dev/full")
}

#[test]
fn non_utf8_program_name_write_errors_match_gnu() {
    let program_names = [
        OsString::from_vec(b"h\xff".to_vec()),
        OsString::from_vec(b"/tmp/h\xff".to_vec()),
    ];

    for locale in ["C", "C.utf8", "zh_CN.utf8"] {
        for program_name in &program_names {
            let gnu = run_hostid("/usr/bin/hostid", locale, program_name);
            let syskits = run_hostid(env!("CARGO_BIN_EXE_hostid"), locale, program_name);

            assert_eq!(
                syskits.status.code(),
                gnu.status.code(),
                "{locale}: {program_name:?}"
            );
            assert_eq!(syskits.stdout, gnu.stdout, "{locale}: {program_name:?}");
            assert_eq!(syskits.stderr, gnu.stderr, "{locale}: {program_name:?}");
        }
    }
}
