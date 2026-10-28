#![cfg(unix)]

use std::fs;
use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn unsorted_input_uses_utility_name_in_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("file1"), b"2 b\n1 a\n3 c\n").unwrap();
    fs::write(directory.path().join("file2"), b"1 x\n2 y\n3 z\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_join"))
        .arg0("join")
        .env("LC_ALL", "C")
        .current_dir(directory.path())
        .args(["file1", "file2"])
        .output()
        .expect("run join");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"2 b y\n3 c z\n");
    assert_eq!(
        output.stderr,
        concat!(
            "join: file1:2: is not sorted: 1 a\n",
            "join: input is not in sorted order\n",
        )
        .as_bytes()
    );
}
