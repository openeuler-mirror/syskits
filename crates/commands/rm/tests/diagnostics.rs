#![cfg(unix)]

use std::fs;
use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn abbreviated_no_preserve_root_has_no_usage_hint() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("victim"), b"data").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rm"))
        .arg0("rm")
        .env("LC_ALL", "C")
        .current_dir(directory.path())
        .args(["--no-preserve-roo", "victim"])
        .output()
        .expect("run rm");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"rm: you may not abbreviate the --no-preserve-root option\n"
    );
    assert!(directory.path().join("victim").is_file());
}

#[test]
fn empty_name_is_treated_as_an_operand() {
    let directory = tempfile::tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rm"))
        .arg0("rm")
        .env("LC_ALL", "C")
        .current_dir(directory.path())
        .arg("")
        .output()
        .expect("run rm");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"rm: cannot remove '': No such file or directory\n"
    );
}
