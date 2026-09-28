use std::fs::{self, File};
use std::os::unix::process::CommandExt;
use std::process::Command;

use tempfile::TempDir;

#[test]
fn parents_failure_names_directory_in_error() {
    let tempdir = TempDir::new().unwrap();
    fs::create_dir_all(tempdir.path().join("a/b")).unwrap();
    File::create(tempdir.path().join("a/keep")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_rmdir"))
        .arg0("rmdir")
        .args(["-p", "a/b"])
        .current_dir(tempdir.path())
        .env("LC_ALL", "C")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"rmdir: failed to remove directory 'a': Directory not empty\n"
    );
    assert!(!tempdir.path().join("a/b").exists());
    assert!(tempdir.path().join("a").exists());
}
