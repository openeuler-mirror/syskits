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
fn bre_anchors_are_special_only_at_branch_edges() {
    for (value, pattern) in [
        ("ab", "^ab"),
        ("ab", "ab$"),
        ("a^b", "a^b"),
        ("a$b", "a$b"),
        ("ab", r"\(^ab\)"),
        ("ab", r"\(ab$\)"),
        ("ab", r"a\|^ab"),
        ("ab", r"ab\|ab$"),
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
