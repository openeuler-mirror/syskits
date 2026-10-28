#![cfg(unix)]

use std::ffi::OsStr;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

fn run_split<I, S>(locale: &str, args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_split"))
        .arg0("split")
        .env("LC_ALL", locale)
        .args(args)
        .output()
        .expect("run split")
}

fn assert_diagnostic(args: &[&str], expected: &str) {
    let output = run_split("C.UTF-8", args);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(String::from_utf8(output.stderr).unwrap(), expected);
}

#[test]
fn localized_operand_quotes_match_gnu() {
    for (args, expected) in [
        (&["-l", "0"][..], "split: invalid number of lines: ‘0’\n"),
        (
            &["-n", "l/1o"][..],
            "split: invalid number of chunks: ‘1o’\n",
        ),
        (
            &["--numeric-suffixes=-1"][..],
            "split: ‘-1’: invalid start value for numerical suffix\n\
Try 'split --help' for more information.\n",
        ),
        (&["-txx"][..], "split: multi-character separator ‘xx’\n"),
    ] {
        assert_diagnostic(args, expected);
    }
}

#[test]
fn usage_hints_match_gnu() {
    assert_diagnostic(
        &["-0"],
        "split: invalid number of lines: ‘0’\n\
Try 'split --help' for more information.\n",
    );

    let input = tempfile::NamedTempFile::new().unwrap();
    let output = run_split(
        "C.UTF-8",
        [
            OsStr::new("-a"),
            OsStr::new("3"),
            OsStr::new("--numeric-suffixes=1000"),
            input.path().as_os_str(),
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "split: numerical suffix start value is too large for the suffix length\n\
Try 'split --help' for more information.\n"
    );

    assert_diagnostic(
        &["-n", "1/2", "--filter=true", "/dev/null"],
        "split: --filter does not process a chunk extracted to stdout\n\
Try 'split --help' for more information.\n",
    );

    for args in [
        &["--numeric-suffixes=100", "--number=r/100", "/dev/null"][..],
        &[
            "--numeric-suffixes=100",
            "--suffix-length=2",
            "--line-bytes=1",
            "/dev/null",
        ][..],
    ] {
        assert_diagnostic(
            args,
            "split: numerical suffix start value is too large for the suffix length\n\
Try 'split --help' for more information.\n",
        );
    }
}

#[test]
fn empty_record_separator_matches_gnu() {
    assert_diagnostic(&["-t", ""], "split: empty record separator\n");
}
