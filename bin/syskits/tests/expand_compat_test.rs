use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn expand_tabs_slash_remaining_step_is_not_checked_as_ascending_stop() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("TZ", "UTC0")
        .args(["expand", "--tabs=3,7,/4"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn syskits expand");

    child
        .stdin
        .as_mut()
        .expect("stdin is piped")
        .write_all(b"\tA\tB\n")
        .expect("write stdin");

    let output = child.wait_with_output().expect("wait for syskits expand");

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"   A   B\n");
    assert_eq!(output.stderr, b"");
}
