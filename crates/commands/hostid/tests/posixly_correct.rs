#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};

fn run_hostid(binary: &str, posixly_correct: &str, args: &[&str]) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("hostid")
        .args(args)
        .stdin(Stdio::null())
        .env("LC_ALL", "C")
        .env("POSIXLY_CORRECT", posixly_correct)
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run hostid")
}

#[test]
fn posixly_correct_option_order_matches_gnu() {
    for posixly_correct in ["", "1"] {
        for args in [
            ["value", "--help"].as_slice(),
            ["value", "--version"].as_slice(),
            ["value", "--invalid"].as_slice(),
            ["value", "-h"].as_slice(),
            ["value", "--invalid", "second"].as_slice(),
            ["--invalid", "value"].as_slice(),
            ["--", "value", "--help"].as_slice(),
        ] {
            let gnu = run_hostid("/usr/bin/hostid", posixly_correct, args);
            let syskits = run_hostid(env!("CARGO_BIN_EXE_hostid"), posixly_correct, args);

            assert_eq!(
                syskits.status.code(),
                gnu.status.code(),
                "POSIXLY_CORRECT={posixly_correct:?}: {args:?}"
            );
            assert_eq!(
                syskits.stdout, gnu.stdout,
                "POSIXLY_CORRECT={posixly_correct:?}: {args:?}"
            );
            assert_eq!(
                syskits.stderr, gnu.stderr,
                "POSIXLY_CORRECT={posixly_correct:?}: {args:?}"
            );
        }
    }
}
