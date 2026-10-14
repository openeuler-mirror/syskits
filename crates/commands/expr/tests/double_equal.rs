#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

fn run_expr(binary: &str, arguments: &[&str]) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("expr")
        .args(arguments)
        .env("LC_ALL", "C")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run expr")
}

#[test]
fn double_equal_comparison_matches_gnu_coreutils() {
    for arguments in [
        ["1", "==", "1"].as_slice(),
        ["1", "==", "2"].as_slice(),
        ["alpha", "==", "alpha"].as_slice(),
        ["alpha", "==", "beta"].as_slice(),
    ] {
        let gnu = run_expr("/usr/bin/expr", arguments);
        let syskits = run_expr(env!("CARGO_BIN_EXE_expr"), arguments);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{arguments:?}");
        assert_eq!(syskits.stdout, gnu.stdout, "{arguments:?}");
        assert_eq!(syskits.stderr, gnu.stderr, "{arguments:?}");
    }
}
