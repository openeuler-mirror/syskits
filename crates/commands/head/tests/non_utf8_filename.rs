#![cfg(target_os = "linux")]

use std::ffi::{OsStr, OsString};
use std::fs::write;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use tempfile::tempdir;

fn run_head(binary: &str, args: &[&OsStr]) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("head")
        .args(args)
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
fn non_utf8_filename_matches_gnu() {
    let tempdir = tempdir().expect("create temporary directory");
    let directory = tempdir.path().as_os_str().as_encoded_bytes();
    let input = PathBuf::from(OsString::from_vec([directory, b"/input-\xff"].concat()));
    write(&input, b"non-utf8 path content\n").expect("write input");

    assert_matches_gnu(&[OsStr::new("--verbose"), input.as_os_str()]);

    let missing = PathBuf::from(OsString::from_vec([directory, b"/missing-\xff"].concat()));
    assert_matches_gnu(&[OsStr::new("--verbose"), missing.as_os_str()]);
}
