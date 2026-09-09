use std::process::Command;

use tempfile::TempDir;

#[test]
fn od_long_strings_without_bytes_keeps_following_operand_as_file() {
    let temp_dir = TempDir::new().expect("tempdir");
    let testfile = temp_dir.path().join("testfile");
    std::fs::write(&testfile, "testfile\n").expect("write testfile");

    let path = testfile.to_str().expect("utf8 path");
    let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["od", "--strings", path])
        .output()
        .expect("run syskits od --strings");
    assert_eq!(syskits.status.code(), Some(0));
    assert!(syskits.stdout.is_empty());
    assert!(syskits.stderr.is_empty());
}

#[test]
fn od_long_width_without_bytes_keeps_following_operand_as_file() {
    let temp_dir = TempDir::new().expect("tempdir");
    let testfile = temp_dir.path().join("testfile");
    std::fs::write(&testfile, "afsafajksa\n").expect("write testfile");

    let path = testfile.to_str().expect("utf8 path");
    let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["od", "--width", path])
        .output()
        .expect("run syskits od --width");
    assert_eq!(syskits.status.code(), Some(0));
    assert_eq!(
        syskits.stdout,
        b"0000000 063141 060563 060546 065552 060563 000012\n0000013\n"
    );
    assert!(syskits.stderr.is_empty());
}

#[test]
fn od_short_width_without_attached_bytes_keeps_following_operand_as_file() {
    let temp_dir = TempDir::new().expect("tempdir");
    let testfile = temp_dir.path().join("testfile");
    std::fs::write(&testfile, "afsafajksa\n").expect("write testfile");

    let path = testfile.to_str().expect("utf8 path");
    let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["od", "-w", path])
        .output()
        .expect("run syskits od -w");
    assert_eq!(syskits.status.code(), Some(0));
    assert_eq!(
        syskits.stdout,
        b"0000000 063141 060563 060546 065552 060563 000012\n0000013\n"
    );
    assert!(syskits.stderr.is_empty());
}
