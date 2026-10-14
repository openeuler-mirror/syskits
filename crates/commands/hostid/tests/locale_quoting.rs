#![cfg(target_os = "linux")]

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};

fn run_hostid(binary: &str, locale: &str, operand: &OsString) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("hostid")
        .arg("--")
        .arg(operand)
        .stdin(Stdio::null())
        .env("LC_ALL", locale)
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run hostid")
}

#[test]
fn utf8_locale_operand_quoting_matches_gnu() {
    let operands = [
        OsString::from("x’y"),
        OsString::from("‘x’"),
        OsString::from("中文"),
        OsString::from_vec(b"x\xe2\x80\x99\xffy".to_vec()),
    ];

    for locale in ["C.utf8", "C.UTF-8", "en_US.utf8", "en_US.UTF-8"] {
        for operand in &operands {
            let gnu = run_hostid("/usr/bin/hostid", locale, operand);
            let syskits = run_hostid(env!("CARGO_BIN_EXE_hostid"), locale, operand);

            assert_eq!(
                syskits.status.code(),
                gnu.status.code(),
                "{locale}: {operand:?}"
            );
            assert_eq!(syskits.stdout, gnu.stdout, "{locale}: {operand:?}");
            assert_eq!(syskits.stderr, gnu.stderr, "{locale}: {operand:?}");
        }
    }
}
