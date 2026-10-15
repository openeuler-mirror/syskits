#![cfg(target_os = "linux")]

use std::fs::{self, OpenOptions};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::time::{Duration, SystemTime};

#[test]
fn cross_device_move_preserves_regular_file_timestamps() {
    let source_dir = tempfile::tempdir().unwrap();
    let destination_dir = match tempfile::tempdir_in("/dev/shm") {
        Ok(directory) => directory,
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
    fs::set_permissions(&source, fs::Permissions::from_mode(0o754)).unwrap();

    let timestamp = SystemTime::UNIX_EPOCH + Duration::from_secs(978_307_506);
    OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap()
        .set_times(
            fs::FileTimes::new()
                .set_accessed(timestamp)
                .set_modified(timestamp),
        )
        .unwrap();
    let source_metadata = fs::metadata(&source).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_mv"))
        .arg0("mv")
        .env("LC_ALL", "C")
        .arg(&source)
        .arg(&destination)
        .output()
        .expect("run mv");

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert!(!source.exists());

    let destination_metadata = fs::metadata(destination).unwrap();
    assert_eq!(destination_metadata.atime(), source_metadata.atime());
    assert_eq!(
        destination_metadata.atime_nsec(),
        source_metadata.atime_nsec()
    );
    assert_eq!(destination_metadata.mtime(), source_metadata.mtime());
    assert_eq!(
        destination_metadata.mtime_nsec(),
        source_metadata.mtime_nsec()
    );
    assert_eq!(
        destination_metadata.permissions().mode() & 0o777,
        source_metadata.permissions().mode() & 0o777
    );
}
