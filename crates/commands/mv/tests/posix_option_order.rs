#![cfg(unix)]

use std::fs;
use std::os::unix::process::CommandExt;
use std::process::Command;

fn run_mv(directory: &std::path::Path, arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mv"))
        .arg0("mv")
        .current_dir(directory)
        .env("LC_ALL", "C")
        .env("POSIXLY_CORRECT", "1")
        .args(arguments)
        .output()
        .expect("run mv")
}

#[test]
fn posixly_correct_stops_option_parsing_after_the_first_operand() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("source"), b"source").unwrap();

    let output = run_mv(directory.path(), &["source", "-v"]);

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert!(!directory.path().join("source").exists());
    assert_eq!(fs::read(directory.path().join("-v")).unwrap(), b"source");
}

#[test]
fn posixly_correct_skips_required_option_values_before_stopping() {
    for option in ["-t", "--target-directory"] {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::write(directory.path().join("source"), b"source").unwrap();
        fs::write(directory.path().join("-v"), b"dash-v").unwrap();

        let output = run_mv(directory.path(), &[option, "target", "source", "-v"]);

        assert!(output.status.success(), "{option}");
        assert!(output.stdout.is_empty(), "{option}");
        assert!(output.stderr.is_empty(), "{option}");
        assert_eq!(fs::read(target.join("source")).unwrap(), b"source");
        assert_eq!(fs::read(target.join("-v")).unwrap(), b"dash-v");
    }
}

#[test]
fn posixly_correct_does_not_consume_optional_update_value_without_equals() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("source"), b"source").unwrap();
    fs::write(directory.path().join("-v"), b"destination").unwrap();

    let output = run_mv(directory.path(), &["--update", "source", "-v"]);

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read(directory.path().join("source")).unwrap(),
        b"source"
    );
    assert_eq!(
        fs::read(directory.path().join("-v")).unwrap(),
        b"destination"
    );
}
