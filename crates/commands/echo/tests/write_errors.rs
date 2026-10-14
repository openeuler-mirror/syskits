#![cfg(target_os = "linux")]

use std::fs::OpenOptions;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

#[test]
fn stdout_write_errors_use_gnu_diagnostics_for_all_output_paths() {
    for arguments in [
        ["value"].as_slice(),
        ["--help"].as_slice(),
        ["--version"].as_slice(),
    ] {
        let full = OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .expect("open /dev/full");
        let output = Command::new(env!("CARGO_BIN_EXE_echo"))
            .arg0("echo")
            .args(arguments)
            .stdout(Stdio::from(full))
            .output()
            .expect("run echo with /dev/full stdout");

        assert_eq!(output.status.code(), Some(1), "arguments={arguments:?}");
        assert_eq!(
            output.stderr,
            b"echo: write error: No space left on device\n"
        );
    }
}
