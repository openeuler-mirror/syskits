use std::fs::OpenOptions;
use std::process::{Command, Stdio};

#[test]
fn pinky_reports_stdout_write_errors() {
    let full = OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full");
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["pinky", "-l", "root"])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdout(Stdio::from(full))
        .output()
        .expect("run syskits pinky");

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("write error"), "stderr: {stderr:?}");
    assert!(!stderr.contains("panicked"), "stderr: {stderr:?}");
}
