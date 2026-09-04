#[cfg(all(unix, feature = "dirname"))]
use std::ffi::OsString;
#[cfg(all(target_os = "linux", feature = "dirname"))]
use std::fs::OpenOptions;
#[cfg(all(unix, feature = "dirname"))]
use std::os::unix::ffi::OsStringExt;
#[cfg(all(target_os = "linux", feature = "dirname"))]
use std::process::Stdio;
#[cfg(all(unix, feature = "dirname"))]
use std::process::{Command, Output};

#[cfg(all(unix, feature = "dirname"))]
fn run_syskits(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_syskits"))
        .arg("dirname")
        .args(args)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .expect("run syskits dirname")
}

#[cfg(all(unix, feature = "dirname"))]
fn stderr_without_program(stderr: &[u8]) -> &[u8] {
    stderr
        .iter()
        .position(|byte| *byte == b':')
        .map_or(stderr, |colon| &stderr[colon + 1..])
}

#[cfg(all(target_os = "linux", feature = "dirname"))]
#[test]
fn dirname_zero_reports_stdout_write_errors() {
    let syskits_full = OpenOptions::new()
        .write(true)
        .open("/dev/full")
        .expect("open /dev/full for syskits");
    let syskits = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .arg("dirname")
        .args(["-z", "/a/b"])
        .env("LC_ALL", "C")
        .stdout(Stdio::from(syskits_full))
        .output()
        .expect("run syskits dirname");

    assert_eq!(syskits.status.code(), Some(1));
    assert!(syskits.stdout.is_empty());
    assert_eq!(
        stderr_without_program(&syskits.stderr),
        b" write error: No space left on device\n"
    );
}
