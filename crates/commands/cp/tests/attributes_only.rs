#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::time::Duration;

use filetime::{FileTime, set_file_times};

fn run_cp(current_dir: &std::path::Path, arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cp"))
        .arg0("cp")
        .env("LC_ALL", "C")
        .current_dir(current_dir)
        .args(arguments)
        .output()
        .expect("run cp")
}

#[test]
fn attributes_only_keeps_existing_destination_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    let destination = directory.path().join("destination");
    fs::write(&source, b"source-data\n").unwrap();
    fs::write(&destination, b"destination-data\n").unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o600)).unwrap();

    let old_atime = FileTime::from_unix_time(946_684_800, 0);
    set_file_times(
        &source,
        old_atime,
        FileTime::from_unix_time(1_577_836_800, 0),
    )
    .unwrap();
    set_file_times(
        &destination,
        old_atime,
        FileTime::from_unix_time(1_893_456_000, 0),
    )
    .unwrap();

    let before = fs::metadata(&destination).unwrap();
    // Keep this effective on filesystems whose ctime resolution is one second.
    std::thread::sleep(Duration::from_millis(1_100));

    let output = run_cp(
        directory.path(),
        &["--attributes-only", "source", "destination"],
    );

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    let after = fs::metadata(&destination).unwrap();
    assert_eq!(after.ino(), before.ino());
    assert_eq!(after.len(), before.len());
    assert_eq!(after.mode(), before.mode());
    assert_eq!(after.atime(), before.atime());
    assert_eq!(after.atime_nsec(), before.atime_nsec());
    assert_eq!(after.mtime(), before.mtime());
    assert_eq!(after.mtime_nsec(), before.mtime_nsec());
    assert_eq!(after.ctime(), before.ctime());
    assert_eq!(after.ctime_nsec(), before.ctime_nsec());
    assert_eq!(fs::read(destination).unwrap(), b"destination-data\n");
}

#[test]
fn attributes_only_new_destination_uses_source_mode() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("source"), b"source-data\n").unwrap();
    fs::set_permissions(
        directory.path().join("source"),
        fs::Permissions::from_mode(0o640),
    )
    .unwrap();

    let output = run_cp(
        directory.path(),
        &["--attributes-only", "source", "destination"],
    );

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    let destination = fs::metadata(directory.path().join("destination")).unwrap();
    assert_eq!(destination.len(), 0);
    assert_eq!(destination.mode() & 0o777, 0o640);
}

#[test]
fn attributes_only_preserve_mode_updates_existing_destination() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("source"), b"source-data\n").unwrap();
    fs::write(directory.path().join("destination"), b"destination-data\n").unwrap();
    fs::set_permissions(
        directory.path().join("source"),
        fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    fs::set_permissions(
        directory.path().join("destination"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();

    let output = run_cp(
        directory.path(),
        &[
            "--attributes-only",
            "--preserve=mode",
            "source",
            "destination",
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    let destination = fs::metadata(directory.path().join("destination")).unwrap();
    assert_eq!(destination.mode() & 0o777, 0o640);
    assert_eq!(
        fs::read(directory.path().join("destination")).unwrap(),
        b"destination-data\n"
    );
}

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

    let output = run_cp(
        directory.path(),
        &["-a", "--attributes-only", "source-link", "destination"],
    );

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
