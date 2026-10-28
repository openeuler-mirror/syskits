#![cfg(unix)]

use std::fs;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Output};

fn run_cp(current_dir: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cp"))
        .arg0("cp")
        .env("LC_ALL", "C")
        .current_dir(current_dir)
        .args(arguments)
        .output()
        .expect("run cp")
}

#[test]
fn operand_diagnostics_match_gnu() {
    let directory = tempfile::tempdir().unwrap();

    for (arguments, expected) in [
        (
            &[][..],
            "cp: missing file operand\nTry 'cp --help' for more information.\n",
        ),
        (
            &["source"][..],
            "cp: missing destination file operand after 'source'\nTry 'cp --help' for more information.\n",
        ),
        (
            &["-T", "first", "second", "third"][..],
            "cp: extra operand 'third'\nTry 'cp --help' for more information.\n",
        ),
    ] {
        let output = run_cp(directory.path(), arguments);

        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            expected,
            "{arguments:?}"
        );
    }
}

#[test]
fn target_directory_diagnostics_match_gnu() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("source-one"), b"one").unwrap();
    fs::write(directory.path().join("source-two"), b"two").unwrap();
    fs::write(directory.path().join("target-file"), b"target").unwrap();

    for (arguments, expected) in [
        (
            &["source-one", "source-two", "missing"][..],
            "cp: target 'missing': No such file or directory\n",
        ),
        (
            &["source-one", "source-two", "target-file"][..],
            "cp: target 'target-file': Not a directory\n",
        ),
        (
            &["-t", "missing", "source-one"][..],
            "cp: target directory 'missing': No such file or directory\n",
        ),
        (
            &["-t", "target-file", "source-one"][..],
            "cp: target directory 'target-file': Not a directory\n",
        ),
    ] {
        let output = run_cp(directory.path(), arguments);

        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            expected,
            "{arguments:?}"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn reflink_failure_diagnostic_matches_gnu() {
    let directory = match tempfile::tempdir_in("/dev/shm") {
        Ok(directory) => directory,
        Err(_) => return,
    };
    fs::write(directory.path().join("source"), b"source").unwrap();

    let output = run_cp(
        directory.path(),
        &["--reflink=always", "source", "destination"],
    );
    if output.status.success() {
        return;
    }

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "cp: failed to clone 'destination' from 'source': Operation not supported\n"
    );
    assert!(!directory.path().join("destination").exists());
}
