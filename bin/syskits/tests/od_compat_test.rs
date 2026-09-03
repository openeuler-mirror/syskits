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
    let gnu = Command::new("/usr/bin/od")
        .args(["--strings", path])
        .output()
        .expect("run GNU od --strings");

    assert_eq!(syskits.status.code(), gnu.status.code());
    assert_eq!(syskits.stdout, gnu.stdout);
    assert_eq!(syskits.stderr, gnu.stderr);
}
