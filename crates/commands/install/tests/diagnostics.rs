#![cfg(unix)]

use std::fs;
use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn no_target_directory_rejects_directory_destination_like_gnu() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("source"), b"content").unwrap();
    fs::create_dir(directory.path().join("target")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_install"))
        .arg0("install")
        .env("LC_ALL", "C")
        .current_dir(directory.path())
        .args(["-T", "source", "target"])
        .output()
        .expect("run install");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"install: cannot overwrite directory 'target' with non-directory\n"
    );
    assert!(directory.path().join("target").is_dir());
}
