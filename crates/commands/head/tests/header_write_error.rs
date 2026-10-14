#![cfg(target_os = "linux")]

use std::ffi::OsStr;
use std::fs::{OpenOptions, write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use tempfile::tempdir;

fn run_head(binary: &str, args: &[&OsStr]) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("head")
        .args(args)
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

fn assert_matches_gnu(args: &[&OsStr]) {
    let gnu = run_head("/usr/bin/head", args);
    let syskits = run_head(env!("CARGO_BIN_EXE_head"), args);

    assert_eq!(syskits.status.code(), gnu.status.code());
    assert_eq!(syskits.stdout, gnu.stdout);
    assert_eq!(syskits.stderr, gnu.stderr);
}

#[test]
fn header_write_error_matches_gnu() {
    assert_matches_gnu(&[OsStr::new("--verbose"), OsStr::new("/dev/null")]);
}

#[test]
fn header_write_error_after_file_error_matches_gnu() {
    let tempdir = tempdir().expect("create temporary directory");
    let input = tempdir.path().join("input");
    write(&input, b"alpha\nbeta\n").expect("write input");

    assert_matches_gnu(&[
        OsStr::new("--verbose"),
        input.as_os_str(),
        tempdir.path().as_os_str(),
    ]);
}
