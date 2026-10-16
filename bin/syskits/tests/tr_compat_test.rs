use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn tr_reverse_range_is_rejected_before_reading_stdin() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("TZ", "UTC0")
        .args(["tr", "z-a", "x"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run syskits tr");

    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"x\n")
        .expect("write stdin");

    let output = child.wait_with_output().expect("wait for syskits tr");

    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "tr: range-endpoints of 'z-a' are in reverse collating sequence order\n"
    );
}
