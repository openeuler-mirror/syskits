#![cfg(target_os = "linux")]

use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn cross_device_move_preserves_special_and_sparse_files() {
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

    let source = source_parent.path().join("src_tree");
    let source_subdirectory = source.join("nested");
    let source_fifo = source_subdirectory.join("pipe");
    let source_sparse = source.join("sparse");
    let destination = destination_parent.path().join("moved");
    fs::create_dir_all(&source_subdirectory).unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o750)).unwrap();
    fs::set_permissions(&source_subdirectory, fs::Permissions::from_mode(0o750)).unwrap();

    let fifo_name = CString::new(source_fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(
        unsafe { ctcore::libc::mkfifo(fifo_name.as_ptr(), 0o644) },
        0
    );

    let sparse_file = fs::File::create(&source_sparse).unwrap();
    sparse_file.set_len(1_048_576).unwrap();
    sparse_file.write_at(b"BEGIN", 0).unwrap();
    sparse_file.write_at(b"END", 1_048_573).unwrap();
    fs::set_permissions(&source_sparse, fs::Permissions::from_mode(0o640)).unwrap();

    let output = Command::new("/usr/bin/timeout")
        .arg0("timeout")
        .args(["--preserve-status", "--signal=TERM", "--kill-after=.5", "5"])
        .arg(env!("CARGO_BIN_EXE_mv"))
        .env("LC_ALL", "C")
        .current_dir(source_parent.path())
        .args(["src_tree", destination.to_str().unwrap()])
        .output()
        .expect("run mv");

    assert!(output.status.success(), "mv failed: {output:?}");
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert!(!source.exists());

    let destination_metadata = fs::metadata(&destination).unwrap();
    assert_eq!(destination_metadata.permissions().mode() & 0o777, 0o750);
    let destination_subdirectory_metadata = fs::metadata(destination.join("nested")).unwrap();
    assert_eq!(
        destination_subdirectory_metadata.permissions().mode() & 0o777,
        0o750
    );

    let destination_fifo_metadata = fs::symlink_metadata(destination.join("nested/pipe")).unwrap();
    assert!(destination_fifo_metadata.file_type().is_fifo());
    assert_eq!(
        destination_fifo_metadata.permissions().mode() & 0o777,
        0o644
    );

    let destination_sparse = destination.join("sparse");
    let destination_sparse_metadata = fs::metadata(&destination_sparse).unwrap();
    assert_eq!(destination_sparse_metadata.len(), 1_048_576);
    assert_eq!(
        destination_sparse_metadata.permissions().mode() & 0o777,
        0o640
    );
    assert!(
        destination_sparse_metadata.blocks() < destination_sparse_metadata.len() / 512,
        "destination file lost its sparse holes"
    );
    let destination_sparse_file = fs::File::open(destination_sparse).unwrap();
    let mut beginning = [0; 5];
    let mut end = [0; 3];
    destination_sparse_file.read_at(&mut beginning, 0).unwrap();
    destination_sparse_file
        .read_at(&mut end, 1_048_573)
        .unwrap();
    assert_eq!(&beginning, b"BEGIN");
    assert_eq!(&end, b"END");
}
