#![cfg(all(unix, target_os = "linux", not(feature = "selinux")))]

use std::fs;
use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn context_option_is_silent_without_selinux_support() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    let destination = directory.path().join("destination");
    fs::write(&source, b"source").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_mv"))
        .arg0("mv")
        .env("LC_ALL", "C")
        .arg("-Z")
        .arg(&source)
        .arg(&destination)
        .output()
        .expect("run mv");

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert!(!source.exists());
    assert_eq!(fs::read(destination).unwrap(), b"source");
}
