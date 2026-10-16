use std::fs;
use std::process::{Command, Output};

use tempfile::TempDir;

fn run_gnu(directory: &TempDir, args: &[&str]) -> Output {
    Command::new("/usr/bin/paste")
        .current_dir(directory.path())
        .env("LC_ALL", "C")
        .args(args)
        .output()
        .expect("run GNU paste")
}

fn run_syskits(directory: &TempDir, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_syskits"))
        .current_dir(directory.path())
        .env("LC_ALL", "C")
        .arg("paste")
        .args(args)
        .output()
        .expect("run syskits paste")
}

#[test]
fn backslash_zero_suppresses_the_delimiter_like_gnu() {
    let directory = TempDir::new().expect("tempdir");
    fs::write(directory.path().join("left"), b"a\nb\n").expect("write left fixture");
    fs::write(directory.path().join("right"), b"1\n2\n").expect("write right fixture");

    let args = ["-d", "\\0", "left", "right"];
    let gnu = run_gnu(&directory, &args);
    let syskits = run_syskits(&directory, &args);

    assert_eq!(syskits.status.code(), gnu.status.code(), "status");
    assert_eq!(syskits.stderr, gnu.stderr, "stderr");
    assert_eq!(syskits.stdout, gnu.stdout, "stdout");
    assert_eq!(syskits.stdout, b"a1\nb2\n");
}

#[test]
fn backslash_zero_suppresses_serial_delimiters_like_gnu() {
    let directory = TempDir::new().expect("tempdir");
    fs::write(directory.path().join("input"), b"a\nb\n").expect("write input fixture");

    let args = ["-s", "-d", "\\0", "input"];
    let gnu = run_gnu(&directory, &args);
    let syskits = run_syskits(&directory, &args);

    assert_eq!(syskits.status.code(), gnu.status.code(), "status");
    assert_eq!(syskits.stderr, gnu.stderr, "stderr");
    assert_eq!(syskits.stdout, gnu.stdout, "stdout");
    assert_eq!(syskits.stdout, b"ab\n");
}
