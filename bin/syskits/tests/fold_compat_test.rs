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
