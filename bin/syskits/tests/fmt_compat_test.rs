use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run_with_stdin(program: &str, args: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(program)
        .env("LC_ALL", "C")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("run {program}: {err}"));

    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(input)
        .expect("write child stdin");
    child.wait_with_output().expect("wait for child")
}

#[test]
fn split_only_uses_gnu_line_breaking_with_indentation() {
    let input = b"  This is a long paragraph with several words.\n";
    let gnu = run_with_stdin("/usr/bin/fmt", &["-s", "-w", "25"], input);
    let syskits = run_with_stdin(
        env!("CARGO_BIN_EXE_syskits"),
        &["fmt", "-s", "-w", "25"],
        input,
    );

    assert_eq!(syskits.status.code(), gnu.status.code(), "status");
    assert_eq!(syskits.stderr, gnu.stderr, "stderr");
    assert_eq!(syskits.stdout, gnu.stdout, "stdout");
    assert_eq!(
        syskits.stdout,
        b"  This is a long\n  paragraph with several\n  words.\n"
    );
}
