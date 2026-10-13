use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run_cut(args: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cut"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cut");

    child
        .stdin
        .take()
        .expect("cut stdin")
        .write_all(input)
        .expect("write cut stdin");
    child.wait_with_output().expect("wait for cut")
}

#[test]
fn last_repeated_input_delimiter_wins() {
    let colon_last = run_cut(&["-d=", "-d:", "-f2"], b"a:b=c\n");
    assert!(colon_last.status.success());
    assert_eq!(colon_last.stdout, b"b=c\n");
    assert!(colon_last.stderr.is_empty());

    let equals_last = run_cut(&["-d:", "-d=", "-f2"], b"a:b=c\n");
    assert!(equals_last.status.success());
    assert_eq!(equals_last.stdout, b"c\n");
    assert!(equals_last.stderr.is_empty());
}
