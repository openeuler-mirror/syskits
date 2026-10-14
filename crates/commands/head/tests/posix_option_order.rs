#![cfg(target_os = "linux")]

use std::ffi::OsStr;
use std::fs::write;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use tempfile::tempdir;

fn run_head(binary: &str, args: &[&OsStr]) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("head")
        .args(args)
        .stdin(Stdio::null())
        .env("POSIXLY_CORRECT", "1")
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
fn posixly_correct_stops_option_parsing_at_first_file() {
    let tempdir = tempdir().expect("create temporary directory");
    let input = tempdir.path().join("input");
    write(&input, b"one\ntwo\nthree\n").expect("write input");

    for args in [
        vec![input.as_os_str(), OsStr::new("-n"), OsStr::new("1")],
        vec![
            OsStr::new("-n"),
            OsStr::new("1"),
            input.as_os_str(),
            OsStr::new("-v"),
        ],
        vec![input.as_os_str(), OsStr::new("--"), OsStr::new("-n")],
        vec![input.as_os_str(), OsStr::new("-1")],
        vec![
            OsStr::new("--l"),
            OsStr::new("1"),
            input.as_os_str(),
            OsStr::new("-v"),
        ],
    ] {
        assert_matches_gnu(&args);
    }
}
