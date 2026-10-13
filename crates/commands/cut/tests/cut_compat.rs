use std::io::Write;
use std::process::{Command, Output, Stdio};

#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
#[cfg(unix)]
use std::os::unix::process::CommandExt;

fn run_cut(args: &[&str], input: &[u8]) -> Output {
    run_cut_in_locale(args, input, "C")
}

fn run_cut_in_locale(args: &[&str], input: &[u8], locale: &str) -> Output {
    run_cut_os_in_locale(args, input, locale)
}

fn run_cut_os_in_locale<S: AsRef<std::ffi::OsStr>>(
    args: &[S],
    input: &[u8],
    locale: &str,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cut"));
    #[cfg(unix)]
    command.arg0("cut");

    let mut child = command
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

#[cfg(unix)]
#[test]
fn input_delimiter_uses_locale_character_rules() {
    let utf8_in_c = run_cut_os_in_locale(
        &[
            OsString::from("-d"),
            OsString::from("\u{4e2d}"),
            OsString::from("-f2"),
        ],
        b"a\xe4\xb8\xadb\n",
        "C",
    );
    assert!(!utf8_in_c.status.success());
    assert!(utf8_in_c.stdout.is_empty());

    let gb18030 = run_cut_os_in_locale(
        &[
            OsString::from("-d"),
            OsString::from_vec(vec![0xd6, 0xd0]),
            OsString::from("-f2"),
        ],
        b"a\xd6\xd0b\xd6\xd0c\n",
        "zh_CN.gb18030",
    );
    assert!(gb18030.status.success());
    assert_eq!(gb18030.stdout, b"b\n");
    assert!(gb18030.stderr.is_empty());
}

#[test]
fn invalid_ranges_use_gnu_diagnostics() {
    let cases = [
        (
            vec!["-b0"],
            "cut: byte/character positions are numbered from 1\n\
             Try 'cut --help' for more information.\n",
        ),
        (
            vec!["-f", "2-0"],
            "cut: invalid decreasing range\n\
             Try 'cut --help' for more information.\n",
        ),
        (
            vec!["-f", "-"],
            "cut: invalid range with no endpoint: -\n\
             Try 'cut --help' for more information.\n",
        ),
        (
            vec!["-b", "--"],
            "cut: invalid byte or character range\n\
             Try 'cut --help' for more information.\n",
        ),
        (
            vec!["-f", "word"],
            "cut: invalid field value 'word'\n\
             Try 'cut --help' for more information.\n",
        ),
    ];

    for (args, expected_stderr) in cases {
        let output = run_cut(&args, b"");
        assert!(!output.status.success(), "args: {args:?}");
        assert!(output.stdout.is_empty(), "args: {args:?}");
        assert_eq!(output.stderr, expected_stderr.as_bytes(), "args: {args:?}");
    }
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
fn byte_no_split_mode_uses_locale_character_units() {
    let second = run_cut_in_locale(&["-b2", "-n"], b"\xe4\xb8\xadX\n", "en_US.utf8");
    assert!(second.status.success());
    assert_eq!(second.stdout, b"X\n");
    assert!(second.stderr.is_empty());

    let first = run_cut_in_locale(&["-b1", "-n"], b"\xe4\xb8\xadX\n", "en_US.utf8");
    assert!(first.status.success());
    assert_eq!(first.stdout, b"\xe4\xb8\xad\n");
    assert!(first.stderr.is_empty());
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
