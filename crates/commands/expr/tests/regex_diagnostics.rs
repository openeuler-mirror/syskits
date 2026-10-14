#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

fn run_expr(binary: &str, pattern: &str) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("expr")
        .args(["]", ":", pattern])
        .env("LC_ALL", "C")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run expr")
}

#[test]
fn character_class_diagnostics_match_gnu_coreutils() {
    for pattern in ["[]", "[a-z-a]", "[[:alpha:]"] {
        let gnu = run_expr("/usr/bin/expr", pattern);
        let syskits = run_expr(env!("CARGO_BIN_EXE_expr"), pattern);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{pattern:?}");
        assert_eq!(syskits.stdout, gnu.stdout, "{pattern:?}");
        assert_eq!(syskits.stderr, gnu.stderr, "{pattern:?}");
    }
}
