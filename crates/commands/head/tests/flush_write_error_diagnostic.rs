#![cfg(target_os = "linux")]

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use tempfile::NamedTempFile;

fn run_head(binary: &str, argument: &str, input: &NamedTempFile) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("head")
        .arg(argument)
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
fn buffered_write_error_diagnostic_matches_gnu() {
    let mut input = NamedTempFile::new().expect("create input");
    input.write_all(b"abc\n").expect("write input");

    for argument in ["--bytes=1", "--lines=1", "--bytes=-1"] {
        let gnu = run_head("/usr/bin/head", argument, &input);
        let syskits = run_head(env!("CARGO_BIN_EXE_head"), argument, &input);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{argument}");
        assert_eq!(syskits.stdout, gnu.stdout, "{argument}");
        assert_eq!(syskits.stderr, gnu.stderr, "{argument}");
    }
}
