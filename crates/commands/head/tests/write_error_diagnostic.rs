#![cfg(target_os = "linux")]

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use tempfile::NamedTempFile;

fn run_head(binary: &str, input: &NamedTempFile) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("head")
        .args(["--bytes=10485760"])
        .arg(input.path())
        .stdout(Stdio::from(
            OpenOptions::new()
                .write(true)
                .open("/dev/full")
                .expect("open /dev/full"),
        ))
        .stdin(Stdio::null())
        .env("LC_ALL", "C")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run head")
}

#[test]
fn direct_write_error_diagnostic_matches_gnu() {
    let mut input = NamedTempFile::new().expect("create input");
    input
        .write_all(&vec![b'x'; 10 * 1024 * 1024])
        .expect("write input");

    let gnu = run_head("/usr/bin/head", &input);
    let syskits = run_head(env!("CARGO_BIN_EXE_head"), &input);

    assert_eq!(syskits.status.code(), gnu.status.code());
    assert_eq!(syskits.stdout, gnu.stdout);
    assert_eq!(syskits.stderr, gnu.stderr);
}
