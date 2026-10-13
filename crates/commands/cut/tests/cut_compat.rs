use std::io::Write;
use std::process::{Command, Output, Stdio};
#[cfg(unix)]
use tempfile::TempDir;

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

#[cfg(target_os = "linux")]
fn run_cut_to_full(args: &[&str], input: &[u8]) -> Output {
    use std::fs::File;

    let mut command = Command::new(env!("CARGO_BIN_EXE_cut"));
    command.arg0("cut");
    let mut child = command
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::from(
            File::options()
                .write(true)
                .open("/dev/full")
                .expect("/dev/full"),
        ))
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
fn mode_selection_uses_gnu_diagnostics() {
    let missing = run_cut(&[], b"");
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(
        missing.stderr,
        b"cut: you must specify a list of bytes, characters, or fields\n\
          Try 'cut --help' for more information.\n"
    );

    for args in [&["-b1", "-b2"][..], &["-b1", "-c1"][..]] {
        let repeated = run_cut(args, b"");
        assert_eq!(repeated.status.code(), Some(1));
        assert_eq!(
            repeated.stderr,
            b"cut: only one type of list may be specified\n\
              Try 'cut --help' for more information.\n"
        );
    }
}

#[test]
fn non_gnu_whitespace_delimiter_options_are_rejected() {
    let cases = [
        (
            vec!["-w", "-f2"],
            "cut: invalid option -- 'w'\nTry 'cut --help' for more information.\n",
        ),
        (
            vec!["--whitespace-delimited", "-f2"],
            "cut: unrecognized option '--whitespace-delimited'\nTry 'cut --help' for more information.\n",
        ),
    ];

    for (args, expected_stderr) in cases {
        let output = run_cut(&args, b"");
        assert_eq!(output.status.code(), Some(1), "args: {args:?}");
        assert!(output.stdout.is_empty(), "args: {args:?}");
        assert_eq!(output.stderr, expected_stderr.as_bytes(), "args: {args:?}");
    }
}

#[test]
fn unknown_options_use_gnu_diagnostics() {
    let cases = [
        (
            vec!["-q", "-f1"],
            "cut: invalid option -- 'q'\nTry 'cut --help' for more information.\n",
        ),
        (
            vec!["-ab1"],
            "cut: invalid option -- 'a'\nTry 'cut --help' for more information.\n",
        ),
        (
            vec!["-nq", "-b1"],
            "cut: invalid option -- 'q'\nTry 'cut --help' for more information.\n",
        ),
        (
            vec!["--no-such", "-f1"],
            "cut: unrecognized option '--no-such'\nTry 'cut --help' for more information.\n",
        ),
        (
            vec!["--no-such=value", "-f1"],
            "cut: unrecognized option '--no-such=value'\nTry 'cut --help' for more information.\n",
        ),
        (
            vec!["--output-delimiterx=value", "-f1"],
            "cut: unrecognized option '--output-delimiterx=value'\nTry 'cut --help' for more information.\n",
        ),
    ];

    for (args, expected_stderr) in cases {
        let output = run_cut(&args, b"");
        assert_eq!(output.status.code(), Some(1), "args: {args:?}");
        assert!(output.stdout.is_empty(), "args: {args:?}");
        assert_eq!(output.stderr, expected_stderr.as_bytes(), "args: {args:?}");
    }
}

#[test]
fn short_help_and_version_options_are_rejected() {
    for (args, invalid) in [
        (vec!["-h"], 'h'),
        (vec!["-V"], 'V'),
        (vec!["-nh", "-b1"], 'h'),
        (vec!["-nV", "-b1"], 'V'),
    ] {
        let output = run_cut(&args, b"");
        assert_eq!(output.status.code(), Some(1), "args: {args:?}");
        assert!(output.stdout.is_empty(), "args: {args:?}");
        assert_eq!(
            output.stderr,
            format!("cut: invalid option -- '{invalid}'\nTry 'cut --help' for more information.\n")
                .as_bytes(),
            "args: {args:?}"
        );
    }
}

#[test]
fn missing_option_arguments_use_gnu_diagnostics() {
    let cases = [
        (
            vec!["-b"],
            "cut: option requires an argument -- 'b'\nTry 'cut --help' for more information.\n",
        ),
        (
            vec!["-c"],
            "cut: option requires an argument -- 'c'\nTry 'cut --help' for more information.\n",
        ),
        (
            vec!["-d"],
            "cut: option requires an argument -- 'd'\nTry 'cut --help' for more information.\n",
        ),
        (
            vec!["-f"],
            "cut: option requires an argument -- 'f'\nTry 'cut --help' for more information.\n",
        ),
        (
            vec!["--bytes"],
            "cut: option '--bytes' requires an argument\nTry 'cut --help' for more information.\n",
        ),
        (
            vec!["--char"],
            "cut: option '--characters' requires an argument\nTry 'cut --help' for more information.\n",
        ),
        (
            vec!["--delim"],
            "cut: option '--delimiter' requires an argument\nTry 'cut --help' for more information.\n",
        ),
        (
            vec!["--f"],
            "cut: option '--fields' requires an argument\nTry 'cut --help' for more information.\n",
        ),
        (
            vec!["--out"],
            "cut: option '--output-delimiter' requires an argument\nTry 'cut --help' for more information.\n",
        ),
    ];

    for (args, expected_stderr) in cases {
        let output = run_cut(&args, b"");
        assert_eq!(output.status.code(), Some(1), "args: {args:?}");
        assert!(output.stdout.is_empty(), "args: {args:?}");
        assert_eq!(output.stderr, expected_stderr.as_bytes(), "args: {args:?}");
    }
}

#[test]
fn unexpected_long_option_arguments_use_gnu_diagnostics() {
    let cases = [
        (
            vec!["--complement=value", "-b1"],
            "cut: option '--complement' doesn't allow an argument\nTry 'cut --help' for more information.\n",
        ),
        (
            vec!["--com=value", "-b1"],
            "cut: option '--complement' doesn't allow an argument\nTry 'cut --help' for more information.\n",
        ),
        (
            vec!["--only-delimited=value", "-f1"],
            "cut: option '--only-delimited' doesn't allow an argument\nTry 'cut --help' for more information.\n",
        ),
        (
            vec!["--zero-terminated=value", "-f1"],
            "cut: option '--zero-terminated' doesn't allow an argument\nTry 'cut --help' for more information.\n",
        ),
        (
            vec!["--help=value"],
            "cut: option '--help' doesn't allow an argument\nTry 'cut --help' for more information.\n",
        ),
        (
            vec!["--version=value"],
            "cut: option '--version' doesn't allow an argument\nTry 'cut --help' for more information.\n",
        ),
    ];

    for (args, expected_stderr) in cases {
        let output = run_cut(&args, b"");
        assert_eq!(output.status.code(), Some(1), "args: {args:?}");
        assert!(output.stdout.is_empty(), "args: {args:?}");
        assert_eq!(output.stderr, expected_stderr.as_bytes(), "args: {args:?}");
    }
}

#[test]
fn hyphen_prefixed_option_values_are_consumed() {
    let delimiter = run_cut(&["--delimiter", "--no-such", "-f1"], b"a\tb\n");
    assert_eq!(delimiter.status.code(), Some(1));
    assert!(delimiter.stdout.is_empty());
    assert_eq!(
        delimiter.stderr,
        b"cut: the delimiter must be a single character\n\
          Try 'cut --help' for more information.\n"
    );

    let output_delimiter = run_cut(&["--output-delimiter", "--", "-f1,2"], b"a\tb\n");
    assert!(output_delimiter.status.success());
    assert_eq!(output_delimiter.stdout, b"a--b\n");
    assert!(output_delimiter.stderr.is_empty());
}

#[test]
fn input_delimiter_requires_field_mode() {
    for args in [&["-d:", "-b1"][..], &["-d", "", "-c1"][..]] {
        let output = run_cut(args, b"");
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            output.stderr,
            b"cut: an input delimiter may be specified only when operating on fields\n\
              Try 'cut --help' for more information.\n"
        );
    }
}

#[test]
fn invalid_input_delimiter_includes_help_hint() {
    for (args, locale) in [
        (vec!["-d", "aa", "-f1"], "C"),
        (vec!["-d", "\u{4e2d}", "-f1"], "C"),
    ] {
        let output = run_cut_in_locale(&args, b"", locale);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            output.stderr,
            b"cut: the delimiter must be a single character\n\
              Try 'cut --help' for more information.\n"
        );
    }
}

#[test]
fn literal_quotes_are_not_treated_as_an_empty_delimiter() {
    let input_delimiter = run_cut(&["-d", "''", "-f1"], b"");
    assert_eq!(input_delimiter.status.code(), Some(1));
    assert_eq!(
        input_delimiter.stderr,
        b"cut: the delimiter must be a single character\n\
          Try 'cut --help' for more information.\n"
    );

    let output_delimiter = run_cut(&["-d:", "--output-delimiter=''", "-f2,3"], b"a:b:c\n");
    assert!(output_delimiter.status.success());
    assert_eq!(output_delimiter.stdout, b"b''c\n");
    assert!(output_delimiter.stderr.is_empty());
}

#[cfg(unix)]
#[test]
fn non_utf8_file_operand_is_processed() {
    let temp_dir = TempDir::new().expect("tempdir");
    let filename = temp_dir
        .path()
        .join(OsString::from_vec(b"cut-\xff".to_vec()));
    std::fs::write(&filename, b"abc\n").expect("write non-UTF-8 file");

    let output = run_cut_os_in_locale(
        &[OsString::from("-b1"), filename.as_os_str().to_os_string()],
        b"",
        "C",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"a\n");
    assert!(output.stderr.is_empty());
}

#[cfg(unix)]
#[test]
fn non_utf8_file_error_uses_gnu_c_locale_quoting() {
    let temp_dir = TempDir::new().expect("tempdir");
    let prefix = temp_dir.path().join("missing-");
    let filename = prefix.with_file_name(OsString::from_vec(b"missing-\xff".to_vec()));

    let output = run_cut_os_in_locale(
        &[OsString::from("-b1"), filename.as_os_str().to_os_string()],
        b"",
        "C",
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        format!(
            "cut: '{}'$'\\377': No such file or directory\n",
            prefix.to_string_lossy()
        )
        .as_bytes()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn final_stdout_flush_uses_gnu_write_error_diagnostic() {
    let output = run_cut_to_full(&["-b1"], b"abc\n");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        output.stderr,
        b"cut: write error: No space left on device\n"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn midstream_stdout_write_failure_is_reported_once() {
    let mut input = vec![b'x'; 64 * 1024];
    input.push(b'\n');

    let output = run_cut_to_full(&["-b1-"], &input);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        output.stderr,
        b"cut: write error: No space left on device\n"
    );
}

#[test]
fn field_mode_merges_overlapping_ranges() {
    let implicit = run_cut(&["-d:", "-f1-2,2-3"], b"a:b:c:d\n");
    assert!(implicit.status.success());
    assert_eq!(implicit.stdout, b"a:b:c\n");
    assert!(implicit.stderr.is_empty());

    let explicit = run_cut(&["-d:", "-f1-2,2-3", "--output-delimiter=,"], b"a:b:c:d\n");
    assert!(explicit.status.success());
    assert_eq!(explicit.stdout, b"a,b,c\n");
    assert!(explicit.stderr.is_empty());
}

#[test]
fn explicit_output_delimiter_does_not_repeat_an_unterminated_last_field() {
    let output = run_cut(&["-d:", "-f1-2,3-4", "--output-delimiter=,"], b"no:newline");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"no,newline\n");
    assert!(output.stderr.is_empty());
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
fn character_mode_matches_gnu_after_an_invalid_multibyte_record_prefix() {
    let output = run_cut_in_locale(&["-c1"], b"A\n\xffX\n", "en_US.utf8");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"A\n\nX\n");
    assert!(output.stderr.is_empty());

    let byte_no_split = run_cut_in_locale(&["-b1-3", "-n"], b"A\n\xffX\n", "en_US.utf8");
    assert!(byte_no_split.status.success());
    assert_eq!(byte_no_split.stdout, b"A\n\nX\n");
    assert!(byte_no_split.stderr.is_empty());
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
