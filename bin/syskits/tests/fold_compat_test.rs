use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn fold_width_zero_is_rejected() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["fold", "-w", "0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run syskits fold");

    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"x\n")
        .expect("write stdin");

    let output = child.wait_with_output().expect("wait for syskits fold");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "stdout should be empty");

    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(
        stderr.contains("fold: invalid number of columns: '0': Numerical result out of range\n"),
        "stderr: {stderr:?}"
    );
}

#[test]
fn fold_bytes_reset_width_after_newline_in_c_locale() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["fold", "-b", "-w", "8"])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("TZ", "UTC0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run syskits fold");

    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"a\n1234567890\n")
        .expect("write stdin");

    let output = child.wait_with_output().expect("wait for syskits fold");

    assert!(output.status.success());
    assert_eq!(output.stdout, b"a\n12345678\n90\n");
    assert!(output.stderr.is_empty());
}
