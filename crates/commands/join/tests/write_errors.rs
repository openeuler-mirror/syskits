use std::fs::{self, File};
use std::process::{Command, Stdio};

use tempfile::tempdir;

#[cfg(target_os = "linux")]
#[test]
fn stdout_write_errors_use_gnu_diagnostic() {
    let temp = tempdir().unwrap();
    let file1 = temp.path().join("file1");
    let file2 = temp.path().join("file2");
    fs::write(&file1, b"a left\n").unwrap();
    fs::write(&file2, b"a right\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_join"))
        .arg(&file1)
        .arg(&file2)
        .stdout(Stdio::from(File::create("/dev/full").unwrap()))
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr)
        .ends_with(": write error: No space left on device\n"));
}
