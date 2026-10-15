#![cfg(target_os = "linux")]

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn debug_output_for_cross_device_regular_file_matches_gnu() {
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

    let output = Command::new(env!("CARGO_BIN_EXE_mv"))
        .arg0("mv")
        .env("LC_ALL", "C")
        .arg("--debug")
        .arg(&source)
        .arg(&destination)
        .output()
        .expect("run mv");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "copied '{}' -> '{}'\ncopy offload: unsupported, reflink: unsupported, sparse detection: no\nremoved '{}'\n",
            source.display(),
            destination.display(),
            source.display()
        )
    );
    assert!(!source.exists());
    assert_eq!(fs::read(destination).unwrap(), b"source");
}
