#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

fn run(binary: &str, program_name: &str, arguments: &[&str]) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0(program_name)
        .args(arguments)
        .env("LC_ALL", "C")
        .env_remove("LANG")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE");
    command.output().expect("run test command")
}

#[test]
fn string_order_operators_match_gnu_errors() {
    for arguments in [
        ["abc", "<", "abd"],
        ["abd", "<", "abc"],
        ["abd", ">", "abc"],
        ["abc", ">", "abd"],
    ] {
        let gnu_test = run("/usr/bin/test", "/usr/bin/test", &arguments);
        let syskits_test = run(env!("CARGO_BIN_EXE_test"), "/usr/bin/test", &arguments);
        assert_eq!(syskits_test.status.code(), gnu_test.status.code());
        assert_eq!(syskits_test.stdout, gnu_test.stdout);
        assert_eq!(syskits_test.stderr, gnu_test.stderr);

        let mut bracket_arguments = arguments.to_vec();
        bracket_arguments.push("]");
        let gnu_bracket = run("/usr/bin/[", "/usr/bin/[", &bracket_arguments);
        let syskits_bracket = run(env!("CARGO_BIN_EXE_test"), "/usr/bin/[", &bracket_arguments);
        assert_eq!(syskits_bracket.status.code(), gnu_bracket.status.code());
        assert_eq!(syskits_bracket.stdout, gnu_bracket.stdout);
        assert_eq!(syskits_bracket.stderr, gnu_bracket.stderr);
    }
}
