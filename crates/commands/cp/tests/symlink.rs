#![cfg(unix)]

use std::fs;
use std::os::unix::fs::symlink;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

#[test]
fn force_symbolic_link_replaces_existing_destination_link() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("source"), b"source").unwrap();
    symlink("source", directory.path().join("destination")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cp"))
        .arg0("cp")
        .env("LC_ALL", "C")
        .current_dir(directory.path())
        .args(["-dsf", "source", "destination"])
        .output()
        .expect("run cp");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read_link(directory.path().join("destination")).unwrap(),
        Path::new("source")
    );
}
