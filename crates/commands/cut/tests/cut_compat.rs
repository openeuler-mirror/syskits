use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run_cut(args: &[&str], input: &[u8]) -> Output {
    run_cut_in_locale(args, input, "C")
}

fn run_cut_in_locale(args: &[&str], input: &[u8], locale: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cut"))
        .args(args)
        .env("LC_ALL", locale)
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
fn character_mode_uses_locale_character_units() {
    let combining = run_cut_in_locale(&["-c2"], b"a\xcc\x81b\n", "en_US.utf8");
    assert!(combining.status.success());
    assert_eq!(combining.stdout, b"\xcc\x81\n");
    assert!(combining.stderr.is_empty());

    let c_locale = run_cut_in_locale(&["-c1"], b"\xe4\xb8\xadX\n", "C");
    assert!(c_locale.status.success());
    assert_eq!(c_locale.stdout, b"\xe4\n");
    assert!(c_locale.stderr.is_empty());

    let invalid = run_cut_in_locale(&["-c1"], b"\xffX\n", "en_US.utf8");
    assert!(invalid.status.success());
    assert_eq!(invalid.stdout, b"\xff\n");
    assert!(invalid.stderr.is_empty());
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
