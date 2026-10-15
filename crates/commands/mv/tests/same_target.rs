#![cfg(unix)]

use std::fs;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

fn run_mv(arguments: &[&str], directory: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mv"))
        .arg0("mv")
        .current_dir(directory)
        .env("LC_ALL", "C")
        .args(arguments)
        .stdin(Stdio::null())
        .output()
        .expect("run mv")
}

#[test]
fn target_directory_rejects_a_source_that_is_already_at_its_destination() {
    for options in [
        &[][..],
        &["-i"][..],
        &["--backup=numbered"][..],
        &["--update=older"][..],
        &["--update=all"][..],
        &["-f"][..],
    ] {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("target")).unwrap();
        fs::write(directory.path().join("target/file"), b"source").unwrap();

        let mut arguments = options.to_vec();
        arguments.extend(["-t", "target", "target/file"]);
        let output = run_mv(&arguments, directory.path());

        assert_eq!(output.status.code(), Some(1), "{options:?}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "mv: 'target/file' and 'target/file' are the same file\n",
            "{options:?}"
        );
        assert_eq!(
            fs::read(directory.path().join("target/file")).unwrap(),
            b"source"
        );
        assert!(!directory.path().join("target/file.~1~").exists());
    }
}

#[test]
fn no_clobber_and_update_none_skip_a_source_already_at_its_destination() {
    for (options, expected_status, expected_stderr) in [
        (&["-n"][..], Some(1), "mv: not replacing 'target/file'\n"),
        (&["--update=none"][..], Some(0), ""),
    ] {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("target")).unwrap();
        fs::write(directory.path().join("target/file"), b"source").unwrap();

        let mut arguments = options.to_vec();
        arguments.extend(["-t", "target", "target/file"]);
        let output = run_mv(&arguments, directory.path());

        assert_eq!(output.status.code(), expected_status, "{options:?}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            expected_stderr,
            "{options:?}"
        );
        assert_eq!(
            fs::read(directory.path().join("target/file")).unwrap(),
            b"source"
        );
    }
}
