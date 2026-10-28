#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{MetadataExt, symlink};
use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn attributes_only_keeps_existing_destination_for_symlink_source() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("source"), b"source").unwrap();
    fs::write(directory.path().join("destination"), b"destination").unwrap();
    fs::hard_link(
        directory.path().join("destination"),
        directory.path().join("destination-link"),
    )
    .unwrap();
    symlink("source", directory.path().join("source-link")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_cp"))
        .arg0("cp")
        .env("LC_ALL", "C")
        .current_dir(directory.path())
        .args(["-a", "--attributes-only", "source-link", "destination"])
        .output()
        .expect("run cp");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        fs::read(directory.path().join("destination")).unwrap(),
        b"destination"
    );
    assert_eq!(
        fs::metadata(directory.path().join("destination"))
            .unwrap()
            .ino(),
        fs::metadata(directory.path().join("destination-link"))
            .unwrap()
            .ino()
    );
}
