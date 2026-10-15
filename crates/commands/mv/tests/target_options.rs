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
fn target_directory_option_diagnostics_match_gnu() {
    for (arguments, expected) in [
        (
            &["-t", "/tmp", "-t", "/tmp"][..],
            "mv: multiple target directories specified\n",
        ),
        (
            &[
                "--target-directory=/tmp",
                "--target-directory=/tmp",
                "source",
            ][..],
            "mv: multiple target directories specified\n",
        ),
        (
            &["-T", "-t", "/tmp", "source"][..],
            "mv: cannot combine --target-directory (-t) and --no-target-directory (-T)\n",
        ),
        (
            &["-T", "-t", "/tmp"][..],
            "mv: missing file operand\nTry 'mv --help' for more information.\n",
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
fn no_target_directory_rejects_extra_operand_like_gnu() {
    let output = run_mv(&["-T", "first", "second", "third"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "mv: extra operand 'third'\nTry 'mv --help' for more information.\n"
    );
}
