use std::io::{Read, Write};
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn run_pr_with_non_utf8_input(args: &[&str]) -> std::process::Output {
    let dir = tempfile::tempdir().expect("create temp dir");
    let input = dir.path().join("input");
    std::fs::write(&input, [b'a', 0x07, 0x80, b'\n']).expect("write non-utf8 input");

    let mut full_args = vec!["pr"];
    full_args.extend_from_slice(args);
    full_args.push(input.to_str().expect("input path is utf-8"));

    Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("TZ", "UTC0")
        .args(full_args)
        .output()
        .expect("run syskits pr")
}

#[test]
fn pr_pages_zero_is_rejected_without_reading_stdin() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("TZ", "UTC0")
        .args(["pr", "--pages=0"])
        .stdin(Stdio::piped())
        .output()
        .expect("run syskits pr --pages=0");

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
        "pr: invalid page range '0'\n"
    );
}

#[test]
fn pr_rejects_zero_page_length_and_width() {
    for (option, diagnostic) in [
        (
            "-l0",
            "pr: '-l PAGE_LENGTH' invalid number of lines: ‘0’: Numerical result out of range\n",
        ),
        (
            "-w0",
            "pr: '-w PAGE_WIDTH' invalid number of characters: ‘0’: Numerical result out of range\n",
        ),
        (
            "-W0",
            "pr: '-W PAGE_WIDTH' invalid number of characters: ‘0’: Numerical result out of range\n",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .env("LC_ALL", "C.UTF-8")
            .env("LANG", "C.UTF-8")
            .args(["pr", "-t", option])
            .stdin(Stdio::null())
            .output()
            .expect("run syskits pr with a zero page dimension");

        assert_eq!(output.status.code(), Some(1), "option {option}");
        assert_eq!(output.stdout, b"", "option {option}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            diagnostic,
            "option {option}"
        );
    }
}

#[test]
fn pr_number_lines_invalid_argument_exits_without_reading_stdin() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .env("LC_ALL", "C.UTF-8")
        .args(["pr", "--number-lines=invalid"])
        .stdin(Stdio::null())
        .output()
        .expect("run syskits pr --number-lines=invalid");

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
        "pr: '-n' extra characters or invalid number in the argument: ‘nvalid’\nTry 'pr --help' for more information.\n"
    );
}

#[test]
fn pr_show_nonprinting_accepts_non_utf8_input() {
    let output = run_pr_with_non_utf8_input(&["-t", "-v"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"a\\007\\200\n");
    assert_eq!(output.stderr, b"");
}

#[test]
fn pr_c_and_v_escape_zero_width_wide_char() {
    for option in ["-c", "-v"] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
            .env("LC_ALL", "C.UTF-8")
            .env("LANG", "C.UTF-8")
            .args(["pr", "-t", option])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run syskits pr");

        child
            .stdin
            .take()
            .expect("open pr stdin")
            .write_all(b"e\xcc\x81\n")
            .expect("write combining character");

        let output = child.wait_with_output().expect("wait for syskits pr");
        assert_eq!(
            output.status.code(),
            Some(0),
            "option {option}, stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"e\\314\\201\n", "option {option}");
        assert_eq!(output.stderr, b"", "option {option}");
    }
}

#[test]
fn pr_without_show_nonprinting_preserves_non_utf8_bytes() {
    let output = run_pr_with_non_utf8_input(&["-t"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout hex: {:02x?}, stderr: {}",
        output.stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, [b'a', 0x07, 0x80, b'\n']);
    assert_eq!(output.stderr, b"");
}

#[test]
fn pr_streams_output_before_stdin_eof() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["pr", "-t"])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run syskits pr");

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

    let input = (1..=20_000)
        .map(|number| format!("{number}\n"))
        .collect::<String>()
        .into_bytes();
    stdin.write_all(&input).expect("write streaming input");
    let first_byte = first_byte_receiver.recv_timeout(Duration::from_secs(2));
    drop(stdin);

    let status = child.wait().expect("wait for syskits pr");
    let (thread_first_byte, remaining_output) = reader.join().expect("join stdout reader");

    assert!(status.success());
    assert_eq!(
        first_byte
            .expect("pr should output before stdin reaches EOF")
            .unwrap(),
        b'1'
    );
    assert_eq!(thread_first_byte, b'1');
    let mut output = vec![thread_first_byte];
    output.extend_from_slice(&remaining_output);
    assert_eq!(output, input);
}

#[cfg(target_os = "linux")]
#[test]
fn pr_accepts_readable_special_file() {
    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["pr", "-t", "/dev/null"])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .expect("run syskits pr");

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn pr_continues_after_file_open_failure() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let missing = dir.path().join("missing");
    let good = dir.path().join("good");
    std::fs::write(&good, b"good\n").expect("write input");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["pr", "-r", "-t"])
        .arg(&missing)
        .arg(&good)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .expect("run syskits pr");

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"good\n");
    assert!(output.stderr.is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn pr_reads_file_with_non_utf8_name() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let file = dir
        .path()
        .join(std::ffi::OsString::from_vec(b"input-\xff".to_vec()));
    std::fs::write(&file, b"raw\n").expect("write input");

    let output = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["pr", "-t"])
        .arg(&file)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::null())
        .output()
        .expect("run syskits pr");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"raw\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn pr_form_feed_keeps_default_body_length() {
    let input = (1..=57)
        .map(|number| format!("{number}\n"))
        .collect::<String>();
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["pr", "-F", "-h", "H", "-D", ""])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run syskits pr");

    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write input");
    let output = child.wait_with_output().expect("wait for syskits pr");
    let first_page = output.stdout.split(|byte| *byte == b'\x0c').next().unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(first_page.iter().filter(|byte| **byte == b'\n').count(), 61);
    assert!(output.stderr.is_empty());
}

#[test]
fn pr_consumes_form_feed_after_full_page() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["pr", "-t", "-l3"])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run syskits pr");

    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"1\n2\n3\n\x0cnext\n")
        .expect("write input");
    let output = child.wait_with_output().expect("wait for syskits pr");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"1\n2\n3\nnext\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn pr_custom_expand_character_also_expands_plain_tabs() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["pr", "-t", "-e.4"])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run syskits pr");

    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"a.b\tc\n")
        .expect("write input");
    let output = child.wait_with_output().expect("wait for syskits pr");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"a   b   c\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn pr_discards_backspace_at_column_zero() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["pr", "-t"])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run syskits pr");

    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"\x08X\n")
        .expect("write input");
    let output = child.wait_with_output().expect("wait for syskits pr");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"X\n");
    assert!(output.stderr.is_empty());
}

#[test]
fn pr_double_space_does_not_double_unterminated_final_line() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_syskits"))
        .args(["pr", "-t", "-d"])
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run syskits pr");

    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"x")
        .expect("write input");
    let output = child.wait_with_output().expect("wait for syskits pr");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"x\n");
    assert!(output.stderr.is_empty());
}
