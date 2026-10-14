#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::process::Command;

fn run_false(args: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_false"));
    command
        .arg0("false")
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .expect("run false")
}

#[test]
fn help_and_version_use_syskits_information() {
    for argument in ["--help", "--version"] {
        let output = run_false(&[argument]);

        assert_eq!(output.status.code(), Some(1), "{argument}");
        assert!(output.stderr.is_empty(), "{argument}");
        assert!(
            output
                .stdout
                .windows(b"syskits".len())
                .any(|window| window == b"syskits"),
            "{argument}"
        );
        assert!(
            !output.stdout.windows(3).any(|window| window == b"GNU"),
            "{argument}"
        );
    }
}

#[test]
fn terminal_options_remain_exact_and_standalone() {
    for args in [
        ["--help", "operand"].as_slice(),
        ["operand", "--help"].as_slice(),
        ["--version", "operand"].as_slice(),
        ["--hel"].as_slice(),
        ["--unknown"].as_slice(),
    ] {
        let output = run_false(args);

        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(output.stderr.is_empty(), "{args:?}");
    }
}
