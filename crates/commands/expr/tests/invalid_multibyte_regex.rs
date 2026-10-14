#![cfg(target_os = "linux")]

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

fn run_expr(binary: &str, arguments: &[OsString]) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("expr")
        .args(arguments)
        .env("LC_ALL", "C.utf8")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run expr")
}

#[test]
fn invalid_multibyte_input_does_not_match_in_utf8_locale() {
    let input = OsString::from_vec(vec![0xce, b'b', b'c', 0xce, 0xb4, b'e', b'f']);
    for pattern in [OsString::from("."), OsString::from(r"\(.\)")] {
        let arguments = vec![input.clone(), OsString::from(":"), pattern];
        let gnu = run_expr("/usr/bin/expr", &arguments);
        let syskits = run_expr(env!("CARGO_BIN_EXE_expr"), &arguments);

        assert_eq!(syskits.status.code(), gnu.status.code(), "{arguments:?}");
        assert_eq!(syskits.stdout, gnu.stdout, "{arguments:?}");
        assert_eq!(syskits.stderr, gnu.stderr, "{arguments:?}");
    }
}
