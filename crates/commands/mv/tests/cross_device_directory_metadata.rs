#![cfg(target_os = "linux")]

use std::fs::{self, File};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::time::{Duration, SystemTime};

fn set_metadata(path: &std::path::Path, mode: u32, timestamp: SystemTime) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    File::open(path)
        .unwrap()
        .set_times(
            fs::FileTimes::new()
                .set_accessed(timestamp)
                .set_modified(timestamp),
        )
        .unwrap();
}

fn assert_metadata_matches(source: &fs::Metadata, destination: &fs::Metadata) {
    assert_eq!(destination.atime(), source.atime());
    assert_eq!(destination.atime_nsec(), source.atime_nsec());
    assert_eq!(destination.mtime(), source.mtime());
    assert_eq!(destination.mtime_nsec(), source.mtime_nsec());
    assert_eq!(
        destination.permissions().mode() & 0o777,
        source.permissions().mode() & 0o777
    );
}

#[test]
fn cross_device_move_preserves_directory_tree_metadata() {
    let source_parent = tempfile::tempdir().unwrap();
    let destination_parent = match tempfile::tempdir_in("/dev/shm") {
        Ok(directory) => directory,
        Err(_) => return,
    };

    if fs::metadata(source_parent.path()).unwrap().dev()
        == fs::metadata(destination_parent.path()).unwrap().dev()
    {
        return;
    }

    let source = source_parent.path().join("source");
    let source_subdirectory = source.join("subdirectory");
    let source_file = source_subdirectory.join("file");
    let destination = destination_parent.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&source_subdirectory).unwrap();
    fs::write(&source_file, b"source").unwrap();
    set_metadata(
        &source,
        0o751,
        SystemTime::UNIX_EPOCH + Duration::from_secs(978_307_506),
    );
    set_metadata(
        &source_subdirectory,
        0o705,
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_015_219_567),
    );
    set_metadata(
        &source_file,
        0o741,
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_049_438_828),
    );
    let source_metadata = fs::metadata(&source).unwrap();
    let source_subdirectory_metadata = fs::metadata(&source_subdirectory).unwrap();
    let source_file_metadata = fs::metadata(&source_file).unwrap();

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
    assert_metadata_matches(&source_metadata, &fs::metadata(&destination).unwrap());
    assert_metadata_matches(
        &source_subdirectory_metadata,
        &fs::metadata(destination.join("subdirectory")).unwrap(),
    );
    assert_metadata_matches(
        &source_file_metadata,
        &fs::metadata(destination.join("subdirectory/file")).unwrap(),
    );
}
