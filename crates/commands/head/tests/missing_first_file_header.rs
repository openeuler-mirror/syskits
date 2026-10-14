#![cfg(target_os = "linux")]

use std::os::unix::process::CommandExt;
use std::process::{Command, Output};

fn run_head(binary: &str, arguments: &[String]) -> Output {
    let mut command = Command::new(binary);
    command
        .arg0("head")
        .args(arguments)
        .env("LC_ALL", "C")
        .env_remove("LC_MESSAGES")
        .env_remove("LC_CTYPE")
        .env_remove("LANG");
    command.output().expect("run head")
}

#[test]
fn missing_first_file_does_not_add_a_header_separator() {
    let missing = format!("/tmp/syskits-head-missing-{}", std::process::id());
    let arguments = vec!["-n".into(), "1".into(), missing, "/etc/hosts".into()];
    let gnu = run_head("/usr/bin/head", &arguments);
    let syskits = run_head(env!("CARGO_BIN_EXE_head"), &arguments);

    assert_eq!(syskits.status.code(), gnu.status.code());
    assert_eq!(syskits.stdout, gnu.stdout);
    assert_eq!(syskits.stderr, gnu.stderr);
}
