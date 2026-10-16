use std::io::{Read, Write};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn run_fold_with_stdin(args: &[&str], locale: &str, input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(args)
        .env("LC_ALL", locale)
        .env("LANG", locale)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run syskits fold");

    {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(input).expect("write stdin");
    }

    child.wait_with_output().expect("wait for syskits fold")
}

fn assert_fold_bytes(case: &str, args: &[&str], locale: &str, input: &[u8], expected: &[u8]) {
    let output = run_fold_with_stdin(args, locale, input);

    assert!(
        output.status.success(),
        "{case}: stderr={:?}",
        output.stderr
    );
    assert_eq!(output.stdout, expected, "{case}");
    assert!(
        output.stderr.is_empty(),
        "{case}: stderr={:?}",
        output.stderr
    );
}

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

#[test]
fn fold_spaces_handles_carriage_return_after_blank() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["fold", "-s", "-w", "1"])
        .env("LC_ALL", "C.UTF-8")
        .env("LANG", "C.UTF-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run syskits fold");

    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"a \r\xe7\x95\x8c")
        .expect("write stdin");

    let output = child.wait_with_output().expect("wait for syskits fold");

    assert!(output.status.success());
    assert_eq!(output.stdout, b"a\n \n\r\xe7\x95\x8c");
    assert!(output.stderr.is_empty());
}

#[test]
fn fold_wraps_before_backspace_when_oversized_tab_still_exceeds_width() {
    assert_fold_bytes(
        "backspace after oversized tab",
        &["fold", "-w", "4"],
        "C.UTF-8",
        b"\t\x08",
        b"\t\n\x08",
    );
}

#[test]
fn fold_wraps_oversized_tab_after_carriage_return_in_c_locale() {
    assert_fold_bytes(
        "oversized tab after carriage return",
        &["fold", "-w", "4"],
        "C",
        b"\r\t",
        b"\r\n\t",
    );
}

#[test]
fn fold_bytes_spaces_do_not_apply_multibyte_carriage_return_compensation() {
    assert_fold_bytes(
        "byte mode carriage return after blank",
        &["fold", "-b", "-s", "-w", "3"],
        "C",
        b" \ra\t",
        b" \n\ra\t",
    );
}

#[test]
fn fold_uses_locale_encoding_like_gnu() {
    assert_fold_bytes(
        "C locale byte mode",
        &["fold", "-b", "-w", "2"],
        "C",
        b"\xe7\x95\x8cx\n",
        b"\xe7\x95\n\x8cx\n",
    );
    assert_fold_bytes(
        "UTF-8 conversion failure",
        &["fold", "-w", "2"],
        "C.UTF-8",
        b"\xc3\xa9a\xffb\n",
        b"\xc3\xa9a\n\xffb\n\n",
    );
    assert_fold_bytes(
        "EUC-JP character mode",
        &["fold", "-c", "-w", "1"],
        "ja_JP.eucjp",
        b"\xa4\xa2\xa4\xa2",
        b"\xa4\xa2\n\xa4\xa2",
    );
}

#[test]
fn fold_streams_output_before_stdin_eof() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["fold", "-w", "1"])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run syskits fold");

    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = child.stdout.take().expect("stdout");
    let (first_byte_sender, first_byte_receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut output = Vec::new();
        let mut first_byte = [0];
        let first_read = stdout.read_exact(&mut first_byte);
        first_byte_sender
            .send(first_read.map(|()| first_byte[0]))
            .expect("send first output byte");
        stdout
            .read_to_end(&mut output)
            .expect("read remaining stdout");
        (first_byte[0], output)
    });

    stdin
        .write_all(&vec![b'x'; 20_000])
        .expect("write streaming input");
    let first_byte = first_byte_receiver.recv_timeout(Duration::from_secs(2));
    drop(stdin);

    let status = child.wait().expect("wait for syskits fold");
    let (thread_first_byte, remaining_output) = reader.join().expect("join stdout reader");

    assert!(status.success());
    assert_eq!(
        first_byte
            .expect("fold should output before stdin reaches EOF")
            .unwrap(),
        b'x'
    );
    assert_eq!(thread_first_byte, b'x');
    assert_eq!(remaining_output.len(), 39_998);
}

#[cfg(unix)]
#[test]
fn fold_continues_after_input_read_error() {
    let temp_dir = tempfile::tempdir().expect("create temp directory");
    let good_file = temp_dir.path().join("good.txt");
    std::fs::write(&good_file, b"content after read error\n").expect("write good input");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["fold", "-w", "100"])
        .arg(temp_dir.path())
        .arg(&good_file)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run syskits fold");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"content after read error\n");
    assert_eq!(
        String::from_utf8(output.stderr).expect("utf8 stderr"),
        format!("fold: {}: Is a directory\n", temp_dir.path().display())
    );
}
