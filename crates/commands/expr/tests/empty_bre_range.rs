#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

fn run_expr(binary: &str, value: &str) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("expr")
        .args([value, ":", "[z-a]"])
        .env("LC_ALL", "C")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run expr")
}

#[test]
fn empty_bre_range_matches_gnu_coreutils() {
    for value in ["", "a", "z", "za"] {
        let gnu = run_expr("/usr/bin/expr", value);
        let syskits = run_expr(env!("CARGO_BIN_EXE_expr"), value);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{value:?}");
        assert_eq!(syskits.stdout, gnu.stdout, "{value:?}");
        assert_eq!(syskits.stderr, gnu.stderr, "{value:?}");
    }
}
