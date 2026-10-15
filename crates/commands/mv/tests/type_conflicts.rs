#![cfg(unix)]

use std::fs;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

fn run_mv(arguments: &[&str], directory: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mv"))
        .arg0("mv")
        .current_dir(directory)
        .env("LC_ALL", "C")
        .args(arguments)
        .output()
        .expect("run mv")
}

#[test]
fn regular_file_cannot_overwrite_directory_in_target_directory() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("source"), b"source").unwrap();
    fs::create_dir(directory.path().join("target")).unwrap();
    fs::create_dir(directory.path().join("target/source")).unwrap();

    let output = run_mv(&["source", "target"], directory.path());

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "mv: cannot overwrite directory 'target/source' with non-directory\n"
    );
    assert!(directory.path().join("source").exists());
    assert!(directory.path().join("target/source").is_dir());
}

#[test]
fn directory_cannot_overwrite_a_nonempty_directory_in_target_directory() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("source")).unwrap();
    fs::create_dir(directory.path().join("target")).unwrap();
    fs::create_dir(directory.path().join("target/source")).unwrap();
    fs::write(directory.path().join("target/source/file"), b"occupied").unwrap();

    let output = run_mv(&["source", "target"], directory.path());

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "mv: cannot overwrite 'target/source': Directory not empty\n"
    );
    assert!(directory.path().join("source").is_dir());
    assert!(directory.path().join("target/source/file").exists());
}

#[test]
fn no_target_directory_preserves_nonempty_directory_diagnostic() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("source")).unwrap();
    fs::create_dir(directory.path().join("target")).unwrap();
    fs::write(directory.path().join("target/file"), b"occupied").unwrap();

    let output = run_mv(&["-T", "source", "target"], directory.path());

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "mv: cannot overwrite 'target': Directory not empty\n"
    );
    assert!(directory.path().join("source").is_dir());
    assert!(directory.path().join("target/file").exists());
}
