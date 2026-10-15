#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::process::Command;

fn run_mv(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mv"))
        .arg0("mv")
        .env("LC_ALL", "C")
        .args(arguments)
        .output()
        .expect("run mv")
}

#[test]
fn target_directory_option_diagnostics_match_gnu() {
    for (arguments, expected) in [
        (
            &["-t", "/tmp", "-t", "/tmp"][..],
            "mv: multiple target directories specified\n",
        ),
        (
            &[
                "--target-directory=/tmp",
                "--target-directory=/tmp",
                "source",
            ][..],
            "mv: multiple target directories specified\n",
        ),
        (
            &["-T", "-t", "/tmp", "source"][..],
            "mv: cannot combine --target-directory (-t) and --no-target-directory (-T)\n",
        ),
        (
            &["-T", "-t", "/tmp"][..],
            "mv: missing file operand\nTry 'mv --help' for more information.\n",
        ),
    ] {
        let output = run_mv(arguments);
        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            expected,
            "{arguments:?}"
        );
    }
}

#[test]
fn no_target_directory_rejects_extra_operand_like_gnu() {
    let output = run_mv(&["-T", "first", "second", "third"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "mv: extra operand 'third'\nTry 'mv --help' for more information.\n"
    );
}

#[test]
fn target_directory_metadata_diagnostics_match_gnu() {
    use std::fs;

    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    let missing = directory.path().join("missing");
    let file = directory.path().join("file");
    fs::write(&source, b"source").unwrap();
    fs::write(&file, b"file").unwrap();

    for (target, reason) in [
        (&missing, "No such file or directory"),
        (&file, "Not a directory"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_mv"))
            .arg0("mv")
            .env("LC_ALL", "C")
            .arg("-t")
            .arg(target)
            .arg(&source)
            .output()
            .expect("run mv");

        assert_eq!(output.status.code(), Some(1), "{}", target.display());
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!("mv: target directory '{}': {reason}\n", target.display()),
            "{}",
            target.display()
        );
        assert!(source.exists());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn no_copy_cross_device_diagnostic_matches_gnu() {
    use std::fs;
    use std::os::unix::fs::MetadataExt;

    let source_dir = tempfile::tempdir().unwrap();
    let destination_dir = match tempfile::tempdir_in("/dev/shm") {
        Ok(dir) => dir,
        Err(_) => return,
    };

    if fs::metadata(source_dir.path()).unwrap().dev()
        == fs::metadata(destination_dir.path()).unwrap().dev()
    {
        return;
    }

    let source = source_dir.path().join("source");
    let destination = destination_dir.path().join("destination");
    fs::write(&source, b"source").unwrap();

    for debug in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_mv"));
        command.arg0("mv").env("LC_ALL", "C");
        if debug {
            command.arg("--debug");
        }
        let output = command
            .arg("--no-copy")
            .arg(&source)
            .arg(&destination)
            .output()
            .expect("run mv");

        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!(
                "mv: cannot move '{}' to '{}': Invalid cross-device link\n",
                source.display(),
                destination.display()
            )
        );
        assert!(source.exists());
        assert!(!destination.exists());
    }
}
