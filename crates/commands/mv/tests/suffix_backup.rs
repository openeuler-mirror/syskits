#![cfg(unix)]

use std::fs;
use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn suffix_option_enables_existing_destination_backups() {
    for option in ["-S", "--suffix"] {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let destination = directory.path().join("destination");
        let backup = directory.path().join("destination.backup");
        fs::write(&source, b"new").unwrap();
        fs::write(&destination, b"old").unwrap();

        let output = Command::new(env!("CARGO_BIN_EXE_mv"))
            .arg0("mv")
            .env("LC_ALL", "C")
            .args([option, ".backup"])
            .arg(&source)
            .arg(&destination)
            .output()
            .expect("run mv");

        assert!(output.status.success(), "{option}");
        assert!(output.stderr.is_empty(), "{option}");
        assert!(!source.exists(), "{option}");
        assert_eq!(fs::read(&destination).unwrap(), b"new", "{option}");
        assert_eq!(fs::read(&backup).unwrap(), b"old", "{option}");
    }
}
