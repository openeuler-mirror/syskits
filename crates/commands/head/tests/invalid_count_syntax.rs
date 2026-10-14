#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};

fn run_head(binary: &str, argument: &str) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("head")
        .arg(argument)
        .stdin(Stdio::null())
        .env("LC_ALL", "C")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run head")
}

#[test]
fn invalid_incomplete_or_trailing_whitespace_count_matches_gnu() {
    for argument in [
        "--bytes=+",
        "--bytes=-",
        "--lines=+",
        "--lines=-",
        "--bytes=1 ",
        "--lines=1 ",
    ] {
        let gnu = run_head("/usr/bin/head", argument);
        let syskits = run_head(env!("CARGO_BIN_EXE_head"), argument);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{argument}");
        assert_eq!(syskits.stdout, gnu.stdout, "{argument}");
        assert_eq!(syskits.stderr, gnu.stderr, "{argument}");
    }
}
