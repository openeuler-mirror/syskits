#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};

fn run_head(binary: &str) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("head")
        .args(["--lines=1", "."])
        .stdin(Stdio::null())
        .env("LC_ALL", "C")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run head")
}

#[test]
fn read_error_diagnostic_matches_gnu() {
    let gnu = run_head("/usr/bin/head");
    let syskits = run_head(env!("CARGO_BIN_EXE_head"));

    assert_eq!(syskits.status.code(), gnu.status.code());
    assert_eq!(syskits.stdout, gnu.stdout);
    assert_eq!(syskits.stderr, gnu.stderr);
}
