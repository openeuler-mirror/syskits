#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

fn run_expr(binary: &str, value: &str, pattern: &str) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("expr")
        .args([value, ":", pattern])
        .env("LC_ALL", "C")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run expr")
}

#[test]
fn gnu_bre_extensions_match_gnu_coreutils() {
    for (value, pattern) in [
        ("a1", r"a\w"),
        ("a!", r"a\W"),
        ("a ", r"a\s"),
        ("a1", r"a\S"),
        ("a", r"\<a\>"),
        ("a", r"\ba\b"),
        ("a", r"\`a"),
        ("a", r"a\'"),
    ] {
        let gnu = run_expr("/usr/bin/expr", value, pattern);
        let syskits = run_expr(env!("CARGO_BIN_EXE_expr"), value, pattern);

        assert_eq!(
            syskits.status.code(),
            gnu.status.code(),
            "{value:?} {pattern:?}"
        );
        assert_eq!(syskits.stdout, gnu.stdout, "{value:?} {pattern:?}");
        assert_eq!(syskits.stderr, gnu.stderr, "{value:?} {pattern:?}");
    }
}
