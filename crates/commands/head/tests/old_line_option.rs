#![cfg(target_os = "linux")]

use std::io::Write;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output};
use tempfile::NamedTempFile;

fn run_head(binary: &str, argument: &str, input: &NamedTempFile) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("head")
        .arg(argument)
        .arg(input.path())
        .env("LC_ALL", "C")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run head")
}

#[test]
fn old_line_option_and_multiplier_order_match_gnu() {
    let mut input = NamedTempFile::new().expect("create input");
    for line in 0..600 {
        writeln!(input, "{line}").expect("write input");
    }

    for argument in ["-1l", "-1bl", "-1lb"] {
        let gnu = run_head("/usr/bin/head", argument, &input);
        let syskits = run_head(env!("CARGO_BIN_EXE_head"), argument, &input);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{argument}");
        assert_eq!(syskits.stdout, gnu.stdout, "{argument}");
        assert_eq!(syskits.stderr, gnu.stderr, "{argument}");
    }
}
