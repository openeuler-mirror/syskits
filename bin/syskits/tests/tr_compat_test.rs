use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run_with_stdin(program: &str, args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(program)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("spawn {program} {args:?}: {err}"));

    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin)
        .expect("write stdin");

    child
        .wait_with_output()
        .unwrap_or_else(|err| panic!("wait {program} {args:?}: {err}"))
}

fn assert_same_output(case: &str, gnu: Output, syskits: Output) {
    assert_eq!(
        syskits.status.code(),
        gnu.status.code(),
        "{case}: GNU stderr: {}, syskits stderr: {}",
        String::from_utf8_lossy(&gnu.stderr),
        String::from_utf8_lossy(&syskits.stderr)
    );
    assert_eq!(
        syskits.stdout, gnu.stdout,
        "{case}: GNU stdout: {:02x?}, syskits stdout: {:02x?}",
        gnu.stdout, syskits.stdout
    );
    assert_eq!(
        syskits.stderr,
        gnu.stderr,
        "{case}: GNU stderr: {}, syskits stderr: {}",
        String::from_utf8_lossy(&gnu.stderr),
        String::from_utf8_lossy(&syskits.stderr)
    );
}

#[test]
fn tr_accepts_aix_c_locale_option() {
    let gnu = run_with_stdin("/usr/bin/tr", &["-A", "a-z", "A-Z"], b"abc");
    let syskits = run_with_stdin(
        env!("CARGO_BIN_EXE_syskits"),
        &["tr", "-A", "a-z", "A-Z"],
        b"abc",
    );

    assert_same_output("AIX C locale option", gnu, syskits);
}

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

#[test]
fn tr_escaped_trailing_backslash_does_not_warn() {
    let gnu = run_with_stdin("/usr/bin/tr", &["\\\\", "/"], b"a\\\n");
    let syskits = run_with_stdin(
        env!("CARGO_BIN_EXE_syskits"),
        &["tr", "\\\\", "/"],
        b"a\\\n",
    );

    assert_same_output("escaped trailing backslash", gnu, syskits);
}
