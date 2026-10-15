#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::process::Command;

fn run_mv(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mv"))
        .arg0("mv")
        .env("LC_ALL", "C")
        .args(arguments)
        .output()
        .expect("run mv")
}

#[test]
fn missing_required_option_value_diagnostics_match_gnu() {
    for (arguments, expected) in [
        (
            &["-t"][..],
            "mv: option requires an argument -- 't'\nTry 'mv --help' for more information.\n",
        ),
        (
            &["--target-directory"][..],
            "mv: option '--target-directory' requires an argument\nTry 'mv --help' for more information.\n",
        ),
        (
            &["--target-d"][..],
            "mv: option '--target-directory' requires an argument\nTry 'mv --help' for more information.\n",
        ),
        (
            &["-S"][..],
            "mv: option requires an argument -- 'S'\nTry 'mv --help' for more information.\n",
        ),
        (
            &["--suffix"][..],
            "mv: option '--suffix' requires an argument\nTry 'mv --help' for more information.\n",
        ),
        (
            &["--suff"][..],
            "mv: option '--suffix' requires an argument\nTry 'mv --help' for more information.\n",
        ),
    ] {
        let output = run_mv(arguments);
        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            expected,
            "{arguments:?}"
        );
    }
}

#[test]
fn target_directory_option_consumes_hyphen_prefixed_value_like_gnu() {
    for arguments in [["-t", "-v"], ["--target-directory", "--verbose"]] {
        let output = run_mv(&arguments);
        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "mv: missing file operand\nTry 'mv --help' for more information.\n",
            "{arguments:?}"
        );
    }
}
